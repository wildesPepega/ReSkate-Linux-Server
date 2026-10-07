// Checks of the protocol, codecs and session rules against the C++ server's behaviour.
use crate::config::{config_error, load_config, load_levels, map_destination, map_label, map_setting, save_config, ServerConfig};
use crate::objects::{ObjectResult, ObjectState};
use crate::party::{PartyBook, PartyResult};
use crate::protocol::*;
use crate::speed::SpeedCheck;
use crate::wire::{decode_wire, encode_wire, encode_wire_bytes, DeltaReceiver, DeltaSender};
use crate::words::{contains_bad_words, mask_bad_words};

const PLAYER: u64 = 76561198000000001;
const OTHER: u64 = 76561198000000002;
const THIRD: u64 = 76561198000000003;
const SERVER: u64 = (1 << 56) | (4 << 52) | 12345;

fn base(kind: u16) -> Packet {
    Packet { kind, sequence: 7, session: 0x1234_5678_9abc_def0, map: 42, epoch: 99, time_us: 1_000_000, source: PLAYER, ..Default::default() }
}

fn pose_packet(sequence: u32, time: u64, x: f32) -> Packet {
    let mut p = base(kind::POSE);
    p.sequence = sequence;
    p.time_us = time;
    p.pose.root.position = [x, 1.0, -3.5];
    for i in 0..40 {
        let mut t = Transform::default();
        t.position = [i as f32 * 0.01 + x * 0.001, 0.5, -0.25];
        let angle = i as f32 * 0.1;
        t.rotation = [0.0, (angle / 2.0).sin(), 0.0, (angle / 2.0).cos()];
        p.pose.skater.push(t);
    }
    p.pose.board.push(Transform { position: [x, 0.1, -3.5], ..Default::default() });
    p.pose.board.push(Transform::default());
    p.pose.board.push(Transform { position: [x + 100.0, 0.1, 0.0], scale: [1.5, 1.5, 1.5], ..Default::default() });
    p
}

#[test]
fn header_layout_matches_the_game() {
    let raw = encode(&base(kind::AWAY), false);
    assert_eq!(raw.len(), PACKET_HEADER_SIZE);
    assert_eq!(&raw[..4], b"RMP1");
    assert_eq!(u16::from_le_bytes([raw[4], raw[5]]), 42);
    assert_eq!(u16::from_le_bytes([raw[6], raw[7]]), kind::AWAY);
    assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 0);
    let p = decode(&raw).unwrap();
    assert_eq!((p.kind, p.session, p.map, p.epoch, p.source, p.world), (kind::AWAY, 0x1234_5678_9abc_def0, 42, 99, PLAYER, 1));
}

#[test]
fn chat_admin_scoring_party_teleport_round_trip() {
    let mut chat = base(kind::CHAT);
    chat.text = "hallo Welt ✓".into();
    assert_eq!(decode(&encode(&chat, false)).unwrap().text, chat.text);

    let mut admin = base(kind::ADMIN);
    admin.text = "map grom".into();
    assert_eq!(decode(&encode(&admin, false)).unwrap().text, "map grom");

    let mut scoring = base(kind::SCORING);
    scoring.scoring = 0xdead_beef;
    scoring.text = "BigAirMod".into();
    let back = decode(&encode(&scoring, false)).unwrap();
    assert_eq!((back.scoring, back.text.as_str()), (0xdead_beef, "BigAirMod"));

    let mut party = base(kind::PARTY);
    party.party_action = party_action::INVITE;
    party.party_player = OTHER;
    let back = decode(&encode(&party, false)).unwrap();
    assert_eq!((back.party_action, back.party_player), (party_action::INVITE, OTHER));

    let mut teleport = base(kind::TELEPORT);
    teleport.teleport = [1.5, -2.0, 300.25];
    assert_eq!(decode(&encode(&teleport, false)).unwrap().teleport, [1.5, -2.0, 300.25]);

    // Invalid text is refused both ways.
    chat.text = "bad\u{7}".into();
    assert!(std::panic::catch_unwind(|| encode(&chat, false)).is_err());
}

#[test]
fn greetings_and_world_messages_round_trip() {
    let mut hello = base(kind::HELLO);
    hello.build = game_sha256_bytes();
    hello.challenge = 5;
    hello.proof = [7; 32];
    hello.text = "Zee".into();
    let back = decode(&encode(&hello, false)).unwrap();
    assert_eq!((back.build, back.challenge, back.proof, back.text.as_str()), (game_sha256_bytes(), 5, [7; 32], "Zee"));
    assert_eq!(encode(&hello, false).len(), PACKET_HEADER_SIZE + 72 + 1 + 3);

    let destination = "Levels/Game/DingoLevel_Root/DingoLevel_Root|Levels/Game/BAM_LevelRoot/BAM_LevelRoot";
    let mut offer = base(kind::MAP_OFFER);
    offer.map = map_hash(destination);
    offer.destination = destination.into();
    offer.map_authorized = true;
    let back = decode(&encode(&offer, false)).unwrap();
    assert!(back.map_authorized);
    assert_eq!(back.destination, destination);

    let mut state = base(kind::WORLD_STATE);
    state.map = map_hash(destination);
    state.destination = destination.into();
    state.world_ready = true;
    state.world = 3;
    let back = decode(&encode(&state, false)).unwrap();
    assert_eq!((back.world, back.world_ready, back.destination.as_str()), (3, true, destination));

    let mut ready = base(kind::WORLD_READY);
    ready.world_ready = true;
    assert!(decode(&encode(&ready, false)).unwrap().world_ready);

    let mut request = base(kind::MAP_REQUEST);
    request.world = 0; // a map request may name no world yet
    assert!(decode(&encode(&request, false)).is_some());
}

#[test]
fn roster_round_trip_and_rules() {
    let mut roster = base(kind::ROSTER);
    roster.source = SERVER;
    roster.capacity = 17;
    roster.members.push(Member { id: SERVER, epoch: 5, name: "My server".into(), ..Default::default() });
    roster.members.push(Member { id: PLAYER, epoch: 6, name: "Zee".into(), admin: true, party: 3, party_leader: true, party_open: true, ..Default::default() });
    roster.members.push(Member { id: OTHER, epoch: 7, name: "Kai".into(), party: 3, speeding: true, scoring: true, ..Default::default() });
    roster.parks = ["skatepark_01".into(), "empty".into(), "streetpark_06".into()];
    roster.server_votes = SERVER_VOTE_MAP | SERVER_VOTE_TIME;
    roster.guest_boosts = false;
    roster.tps = 60;
    roster.voice_range = 250.0;
    let back = decode(&encode(&roster, false)).unwrap();
    assert_eq!(back.members, roster.members);
    assert_eq!(back.parks, roster.parks);
    assert_eq!((back.capacity, back.tps, back.server_votes, back.guest_boosts, back.voice_range), (17, 60, 5, false, 250.0));

    // A party needs one leader and two members; a server is in no party.
    let mut lonely = roster.clone();
    lonely.members.truncate(2);
    assert!(std::panic::catch_unwind(|| encode(&lonely, false)).is_err());
    assert!(game_server_steam_id(SERVER) && !individual_steam_id(SERVER));
    assert!(individual_steam_id(PLAYER));
}

#[test]
fn bans_maps_objects_round_trip() {
    let mut bans = base(kind::BANS);
    bans.ban_total = 3;
    bans.bans.push(Ban { id: PLAYER, name: "Griefer".into(), added: 1_700_000_000 });
    let back = decode(&encode(&bans, false)).unwrap();
    assert_eq!((back.ban_total, back.bans[0].id, back.bans[0].name.as_str(), back.bans[0].added), (3, PLAYER, "Griefer", 1_700_000_000));

    let mut maps = base(kind::MAPS);
    maps.maps = vec!["Levels/Game/BAM_LevelRoot/BAM_LevelRoot".into(), "Levels/Custom/bbcity".into()];
    assert_eq!(decode(&encode(&maps, false)).unwrap().maps, maps.maps);

    let mut objects = base(kind::OBJECTS);
    objects.objects = ObjectChunk {
        base: 0,
        revision: 2,
        part: 0,
        parts: 1,
        objects: vec![NetworkObject { id: 9, item: "own_bk_rail_01".into(), position: [1.0, 2.0, 3.0], ..Default::default() }],
        removed: vec![],
    };
    let back = decode(&encode(&objects, false)).unwrap();
    assert_eq!(back.objects.objects, objects.objects.objects);
}

#[test]
fn compact_poses_keep_their_shape() {
    let p = pose_packet(1, 2_000_000, 10.0);
    let raw = encode(&p, true);
    assert_eq!(u16::from_le_bytes([raw[6], raw[7]]), 8); // packed pose
    let back = decode(&raw).unwrap();
    assert_eq!(back.kind, kind::POSE);
    assert_eq!(back.pose.skater.len(), 40);
    assert_eq!(back.pose.board.len(), 3);
    for (a, b) in back.pose.skater.iter().zip(&p.pose.skater) {
        for i in 0..3 {
            assert!((a.position[i] - b.position[i]).abs() < 0.001);
        }
        for i in 0..4 {
            assert!((a.rotation[i] - b.rotation[i]).abs() < 0.0002);
        }
    }
    // Wide positions and scales are kept exactly.
    assert_eq!(back.pose.board[2].position[0], 110.0);
    assert_eq!(back.pose.board[2].scale, [1.5; 3]);
    // Full-width poses decode too.
    let full = encode(&p, false);
    assert_eq!(full.len(), PACKET_HEADER_SIZE + 46 + 43 * 40);
    assert_eq!(decode(&full).unwrap().pose.skater[3], p.pose.skater[3]);
}

#[test]
fn wire_compression_round_trips() {
    let p = pose_packet(1, 2_000_000, 1.0);
    let raw = encode(&p, true);
    let wire = encode_wire_bytes(&raw);
    assert!(raw.len() >= 256);
    assert!(wire.starts_with(b"RMC1"));
    assert!(wire.len() < raw.len());
    assert_eq!(crate::wire::decode_wire_bytes(&wire).unwrap(), raw);
    let small = encode_wire(&base(kind::AWAY));
    assert!(small.starts_with(b"RMP1"));
    assert_eq!(decode_wire(&small).unwrap().kind, kind::AWAY);
}

#[test]
fn deltas_reach_the_receiver_exactly() {
    let mut sender = DeltaSender::default();
    let mut receiver = DeltaReceiver::default();
    let mut missing = false;
    // The first pose is a full snapshot that becomes the reference.
    let first = pose_packet(1, 2_000_000, 1.0);
    let update = sender.prepare(&first);
    assert!(update.establishes_baseline());
    assert!(update.bytes.starts_with(b"RMB1"));
    let got = receiver.receive(&update.bytes, &mut missing, 1).unwrap();
    assert_eq!(encode(&got, true), encode(&first, true));
    sender.sent(&first, update);
    // Later poses are sparse patches against it.
    for step in 1..10u32 {
        let next = pose_packet(1 + step, 2_000_000 + u64::from(step) * 33_333, 1.0 + step as f32 * 0.05);
        let update = sender.prepare(&next);
        assert!(!update.establishes_baseline());
        assert!(update.bytes.starts_with(b"RMS1"), "step {step}");
        let got = receiver.receive(&update.bytes, &mut missing, 1).unwrap();
        assert!(!missing);
        assert_eq!(encode(&got, true), encode(&next, true));
        sender.sent(&next, update);
    }
    // A receiver without the reference reports a missing frame, not an error.
    let mut fresh = DeltaReceiver::default();
    let next = pose_packet(20, 2_500_000, 2.0);
    let update = sender.prepare(&next);
    assert!(fresh.receive(&update.bytes, &mut missing, 1).is_none());
    assert!(missing);
}

#[test]
fn cosmetics_and_audio_use_xor_deltas() {
    let mut p = base(kind::COSMETICS);
    let recipe = |key: u32, version: u32| CosmeticRecipe {
        key,
        version,
        scalars: vec![1, 2, 3],
        items: (1..=12)
            .map(|slot| CosmeticSlot { slot, asset: format!("Own_TopShirt_Gen_TshirtRelaxed_{slot:05}").into_bytes(), parameters: vec![slot; 6] })
            .collect(),
    };
    p.appearance = Appearance { skater: recipe(SKATER_RECIPE_KEY, 2), board: recipe(BOARD_RECIPE_KEY, 1), card: PlayerCard { background: 1, emblem: 2, title: 3 }, ..Default::default() };
    let mut sender = DeltaSender::default();
    let mut receiver = DeltaReceiver::default();
    let mut missing = false;
    let first = sender.prepare(&p);
    assert_eq!(receiver.receive(&first.bytes, &mut missing, 1).unwrap().appearance, p.appearance);
    sender.sent(&p, first);
    p.sequence += 1;
    p.time_us += 10_000_000; // cosmetics references never expire
    p.appearance.card.title = 9;
    let second = sender.prepare(&p);
    assert!(second.bytes.starts_with(b"RMD1"));
    assert_eq!(receiver.receive(&second.bytes, &mut missing, 1).unwrap().appearance, p.appearance);

    let mut audio = base(kind::AUDIO);
    for age in [30_000u32, 20_000, 0] {
        let mut s = AudioSample { age_us: age, ..Default::default() };
        s.state.values[3] = age as f32;
        s.state.selectors[2] = 4;
        s.state.flags[5] = 1;
        s.event = age == 0;
        audio.audio.push(s);
    }
    let back = decode(&encode(&audio, true)).unwrap();
    assert_eq!(back.audio.len(), 3);
    assert_eq!(back.audio[1].state, audio.audio[1].state);
    assert!(back.audio[2].event);
}

#[test]
fn chat_text_rules() {
    assert_eq!(clean_chat_text("  hi\tthere\n "), "hi there");
    assert_eq!(clean_chat_text(&"ä".repeat(150)).len(), 200);
    assert!(valid_chat_text("ok".as_bytes()));
    assert!(!valid_chat_text("   ".as_bytes()));
    assert!(!valid_chat_text(&[0xC2, 0x85]));
    assert!(!valid_member_name(&[0xff]));
    assert_eq!(format_invite(PLAYER, 0xabc), "76561198000000001-0000000000000abc");
    assert!(newer_sequence(1, u32::MAX));
    assert!(!newer_sequence(5, 5));
    assert_eq!(map_hash("Levels\\Game"), map_hash("levels/game"));
}

#[test]
fn bad_words() {
    assert!(contains_bad_words("fuck"));
    assert!(contains_bad_words("Sh1t"));
    assert!(contains_bad_words("f u c k"));
    assert!(contains_bad_words("fuckface"));
    assert!(!contains_bad_words("Scunthorpe"));
    assert!(!contains_bad_words("cocktail party"));
    assert!(!contains_bad_words("hello world"));
    assert!(!contains_bad_words("ReSkate server"));
    assert_eq!(mask_bad_words("hey shit!"), "hey ****!");
}

#[test]
fn parties() {
    let mut book = PartyBook::new(8);
    assert_eq!(book.invite(PLAYER, OTHER, 0), PartyResult::Ok);
    assert_eq!(book.invite(PLAYER, OTHER, 0), PartyResult::Renewed);
    assert_eq!(book.accept(OTHER, PLAYER, 1), PartyResult::Ok);
    let party = book.party_of(PLAYER);
    assert!(party != 0 && party == book.party_of(OTHER));
    assert_eq!(book.party(party).unwrap().leader, PLAYER);
    assert_eq!(book.join(THIRD, PLAYER, 2), PartyResult::Closed);
    assert_eq!(book.set_open(PLAYER, true), PartyResult::Ok);
    assert_eq!(book.join(THIRD, PLAYER, 2), PartyResult::Ok);
    assert_eq!(book.promote(PLAYER, THIRD), PartyResult::Ok);
    assert_eq!(book.kick(PLAYER, OTHER), PartyResult::NotLeader);
    book.remove(THIRD); // the leader leaves the server: the longest-standing member leads
    assert_eq!(book.party(party).unwrap().leader, PLAYER);
    assert_eq!(book.leave(OTHER), PartyResult::Ok); // one left alone: dissolved
    assert_eq!(book.party_of(PLAYER), 0);
    assert_eq!(book.invite(PLAYER, OTHER, 0), PartyResult::Ok);
    assert_eq!(book.expire(crate::party::INVITE_LIFETIME_US).len(), 1);
}

#[test]
fn speed_check_flags_a_fast_clock_only() {
    let mut normal = SpeedCheck::default();
    let mut fast = SpeedCheck::default();
    let mut flagged = false;
    for i in 0..2000u64 {
        let arrived = 10_000_000 + i * 33_333;
        normal.sample(5_000_000 + i * 33_333 + (i % 7) * 900, arrived);
        if fast.sample(5_000_000 + (i as f64 * 33_333.0 * 1.3) as u64, arrived) && fast.flagged() {
            flagged = true;
        }
    }
    assert!(!normal.flagged());
    assert!((normal.speed() - 1.0).abs() < 0.01);
    assert!(flagged);
    assert!(fast.speed() > 1.25);
}

#[test]
fn object_states_apply_whole_revisions() {
    let objects: Vec<NetworkObject> = (1..=70)
        .map(|id| NetworkObject { id, item: format!("own_bk_box_{id}"), position: [id as f32, 0.0, 0.0], ..Default::default() })
        .collect();
    let mut owner = ObjectState::default();
    owner.replace(&objects);
    let chunks = owner.updates(0);
    assert_eq!(chunks.len(), 2);
    let mut copy = ObjectState::default();
    assert!(copy.receive(&chunks[0]) == ObjectResult::Pending);
    assert!(copy.receive(&chunks[1]) == ObjectResult::Applied);
    assert_eq!(copy.layout(), objects);
    owner.replace(&objects[..69]);
    let delta = owner.updates(copy.revision());
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0].removed, vec![70]);
    assert!(copy.receive(&delta[0]) == ObjectResult::Applied);
    assert_eq!(copy.layout().len(), 69);
}

#[test]
fn password_proofs_depend_on_every_input() {
    let key = crate::password::password_key("hunter2", 77).unwrap();
    let proof = crate::password::password_proof(&key, 77, 1, 2, 3, 4, 5, 6);
    assert!(crate::password::proof_matches(&proof, &crate::password::password_proof(&key, 77, 1, 2, 3, 4, 5, 6)));
    assert!(!crate::password::proof_matches(&proof, &crate::password::password_proof(&key, 77, 1, 2, 3, 4, 5, 7)));
    assert_ne!(key, crate::password::password_key("hunter2", 78).unwrap());
}

#[test]
fn config_file_round_trip_and_maps() {
    let folder = std::env::temp_dir().join(format!("reskate-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mods = folder.join("Mods").join("bbcity");
    std::fs::create_dir_all(&mods).unwrap();
    std::fs::write(mods.join("reskate-levels.json"), r#"{"levels":[{"asset":"Levels/Custom/BBCity/BBCity","displayName":"bbcity"}]}"#).unwrap();
    assert!(load_levels(&folder.join("Mods")).is_empty());
    assert_eq!(map_setting("isle"), "Isle of Grom"); // the start of a name, as in the C++ server
    assert_eq!(map_label("San Vansterdam"), "San Vansterdam");
    assert_eq!(map_setting("bbcity"), "bbcity");
    assert_eq!(map_destination("bbcity"), "Levels/Game/DingoLevel_Root/DingoLevel_Root|Levels/Custom/BBCity/BBCity");
    assert_eq!(map_setting("Levels/Game/DingoLevel_Root/DingoLevel_Root|Levels/Game/DingoLevel_MPR/DingoLevel_MPR"), "Super Ultra Mega Resort");
    assert_eq!(map_setting("Stadium"), "Stadium"); // ambiguous start: kept as typed

    let file = folder.join("ReSkateServer.json");
    let mut added = Vec::new();
    let mut config = load_config(&file, &mut added).unwrap();
    assert!(file.exists());
    assert_eq!(config_error(&config), "");
    config.name = "Linux test".into();
    config.admins.push(PLAYER);
    config.votes.map.enabled = true;
    save_config(&config).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"76561198000000001\""));
    // An older file without newer settings gets them written back.
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("party_size");
    std::fs::write(&file, value.to_string()).unwrap();
    let back = load_config(&file, &mut added).unwrap();
    assert_eq!(added, vec!["party_size".to_string()]);
    assert_eq!(back.name, "Linux test");
    assert_eq!(back.admins, vec![PLAYER]);
    assert!(back.votes.map.enabled);
    assert!(back.status_enabled && back.status_players);
    let mut value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    value["status"]["players"] = false.into();
    std::fs::write(&file, value.to_string()).unwrap();
    let quiet = load_config(&file, &mut added).unwrap();
    assert!(quiet.status_enabled && !quiet.status_players);
    let bad = ServerConfig { port: 5, query_port: 5, ..back.clone() };
    assert_eq!(config_error(&bad), "--port and --query-port must differ.");
    // Ports in an older file are dropped from it, and reported.
    let mut value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(value.get("port").is_none() && value["discord"]["webhook"] == "");
    value.as_object_mut().unwrap().insert("port".into(), 25570.into());
    value.as_object_mut().unwrap().insert("query_port".into(), 25571.into());
    value["discord"]["events"] = serde_json::json!(["join", "Leave"]);
    std::fs::write(&file, value.to_string()).unwrap();
    let back = load_config(&file, &mut added).unwrap();
    assert_eq!(back.dropped, vec!["port = 25570".to_string(), "query_port = 25571".to_string()]);
    assert_eq!((back.port, back.query_port), (27015, 27016));
    assert_eq!(back.discord_events, vec!["join".to_string(), "leave".to_string()]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("\"port\"") && !text.contains("query_port"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn receive_budget_survives_a_backlog_but_not_a_flood() {
    let limit = (2 * u64::from(TICK_RATES[3]) + 80 + 32) as usize;
    // A backlog after a network hiccup: ten times a second's worth in one go, then normal traffic.
    let mut budget = ReceiveBudget::default();
    for _ in 0..limit * 10 {
        assert!(budget.accept(5_000_000, 100, 1));
    }
    for second in 1..10u64 {
        for i in 0..60u64 {
            assert!(budget.accept(5_000_000 + second * 1_000_000 + i * 16_000, 100, 1));
        }
    }
    // A flood that keeps going is dropped in its fourth second.
    let mut flood = ReceiveBudget::default();
    let mut dropped_at = None;
    'outer: for second in 0..10u64 {
        for i in 0..limit as u64 + 50 {
            if !flood.accept(1_000_000 + second * 1_000_000 + i * 1_000, 100, 1) {
                dropped_at = Some(second);
                break 'outer;
            }
        }
    }
    assert_eq!(dropped_at, Some(u64::from(RECEIVE_OVER_SECONDS) - 1));
    assert!(!ReceiveBudget::default().accept(0, 1, 0));
}

fn plugin_snapshot() -> crate::plugins::Snapshot {
    use crate::plugins::{PlayerInfo, Snapshot};
    Snapshot {
        players: vec![
            PlayerInfo { id: 76561198000000001, name: "Alice".into(), admin: true, online_seconds: 600 },
            PlayerInfo { id: 76561198000000002, name: "Bob".into(), admin: false, online_seconds: 30 },
        ],
        server: "Test Server".into(),
        map: "San Vansterdam".into(),
        max_players: 16,
        ..Default::default()
    }
}

fn plugin_text(actions: &[crate::plugins::Action]) -> Vec<String> {
    use crate::plugins::Action;
    actions
        .iter()
        .map(|a| match a {
            Action::Broadcast(t) => format!("all: {t}"),
            Action::Tell(id, t) => format!("{id}: {t}"),
            Action::Log(t) => format!("log: {t}"),
            Action::Run(p, c) => format!("run {p}: {c}"),
        })
        .collect()
}

#[test]
fn plugins_add_commands_messages_and_events() {
    use crate::plugins::PluginManager;
    let folder = std::env::temp_dir().join(format!("reskate-plugins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("folder-plugin")).unwrap();
    for example in ["info", "automessages", "greeter", "chat-filter"] {
        std::fs::copy(format!("examples/plugins/{example}.lua"), folder.join(format!("{example}.lua"))).unwrap();
    }
    std::fs::copy("examples/plugins/_reskate.d.lua", folder.join("_reskate.d.lua")).unwrap(); // skipped
    std::fs::write(folder.join("folder-plugin/main.lua"), "reskate.after(5, function() reskate.run('say hi') end)").unwrap();
    std::fs::write(folder.join("broken.lua"), "this is not lua").unwrap();
    std::fs::write(folder.join("reserved.lua"), "reskate.command('kick', function() end)").unwrap();
    std::fs::write(folder.join("forever.lua"), "reskate.command('spin', function() while true do end end)").unwrap();
    std::fs::write(folder.join("_off.lua"), "reskate.command('off', function() end)").unwrap();

    let start = 1_000_000_000u64;
    let mut plugins = PluginManager::default();
    let lines = plugins.load(&folder, start);
    let all = lines.join("\n");
    assert!(all.contains("broken failed to load"), "{all}");
    assert!(all.contains("reserved failed to load") && all.contains("/kick is a server command"), "{all}");
    assert!(all.contains("6 loaded"), "{all}");
    assert!(!all.contains("/off"), "{all}");

    let snapshot = plugin_snapshot();
    let (alice, bob) = (snapshot.players[0].clone(), snapshot.players[1].clone());
    let (_, reply) = plugins.command(snapshot.clone(), &bob, "discord", "").unwrap();
    assert!(reply.starts_with("Join us on Discord"));
    let (_, reply) = plugins.command(snapshot.clone(), &bob, "r", "").unwrap();
    assert!(reply.starts_with("1. Be nice"));
    let (_, reply) = plugins.command(snapshot.clone(), &bob, "info", "").unwrap();
    assert_eq!(reply, "Hi Bob! You're on Test Server, map San Vansterdam, 2/16 players.");
    assert!(plugins.command(snapshot.clone(), &bob, "nothing", "").is_none());

    // Admin-only commands and /help.
    let (actions, reply) = plugins.command(snapshot.clone(), &bob, "announce", "hello").unwrap();
    assert!(actions.is_empty() && reply == "Only admins can use /announce.");
    let (actions, reply) = plugins.command(snapshot.clone(), &alice, "announce", "hello all").unwrap();
    assert_eq!(plugin_text(&actions), ["all: [Announcement] hello all"]);
    assert!(reply.is_empty());
    assert!(plugins.help(false).contains("/discord") && !plugins.help(false).contains("/announce"));
    assert!(plugins.help(true).contains("/announce"));
    assert_eq!(plugins.describe("announce", true).unwrap(), "/announce <text>: Announce something to everyone");
    assert!(plugins.describe("announce", false).is_none());

    // An endless loop is stopped and reported, and the server carries on.
    let (actions, reply) = plugins.command(snapshot.clone(), &bob, "spin", "").unwrap();
    assert!(reply.contains("failed"));
    assert!(plugin_text(&actions)[0].contains("took too long"));

    // Events.
    let actions = plugins.join(snapshot.clone(), &bob);
    assert_eq!(plugin_text(&actions), [format!("{}: Welcome, Bob! Type /online to see who is here.", bob.id)]);
    let (actions, allowed) = plugins.chat(snapshot.clone(), &bob, "hello BADWORD1");
    assert!(!allowed && plugin_text(&actions).len() == 1);
    assert!(plugins.chat(snapshot.clone(), &bob, "hello").1);

    // Timers and automatic messages.
    assert!(!plugins.due(start + 1_000_000));
    assert!(plugins.due(start + 5_000_000));
    let actions = plugins.tick(start + 5_000_000, snapshot.clone());
    assert_eq!(plugin_text(&actions), ["run folder-plugin: say hi"]);
    assert!(!plugins.due(start + 6_000_000)); // "after" runs once
    let actions = plugins.tick(start + 600_000_000, snapshot.clone());
    assert_eq!(plugin_text(&actions), ["all: Welcome to Test Server! Type /help for the server's commands."]);
    let actions = plugins.tick(start + 1_200_000_000, snapshot.clone());
    assert_eq!(plugin_text(&actions), ["all: Join our Discord: type /discord"]);
    // min_players = 2: quiet with one player on.
    let mut alone = snapshot.clone();
    alone.players.truncate(1);
    let actions = plugins.tick(start + 1_800_000_000, alone);
    assert_eq!(plugin_text(&actions), ["log: [greeter] 1 players online"]); // greeter's 30-minute timer, no message

    // Reload keeps working.
    std::fs::remove_file(folder.join("forever.lua")).unwrap();
    assert!(plugins.reload().join("\n").contains("5 loaded"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn steam_end_reasons_read_as_words() {
    use crate::steam::ended_text;
    assert_eq!(ended_text(4, 1000, "Left the session"), "Steam 1000, closed by the player's game, closed by player: Left the session");
    assert_eq!(ended_text(5, 4001, ""), "Steam 4001, timed out: the player stopped answering, problem detected");
    assert!(ended_text(5, 3003, "").contains("server: lost its Steam relay"));
    assert!(ended_text(5, 5999, "x").ends_with("connection problem, problem detected: x"));
}

#[test]
fn release_versions_compare_with_revisions() {
    use crate::update::{newer, parse_version};
    assert_eq!(parse_version("v1.0.8-1"), Some((vec![1, 0, 8], 1)));
    assert_eq!(parse_version("1.0.8"), Some((vec![1, 0, 8], 0)));
    assert_eq!(parse_version("latest"), None);
    assert!(newer("v1.0.8", "1.0.5"));
    assert!(newer("v1.0.8-1", "1.0.8"));
    assert!(newer("v1.0.10", "1.0.9"));
    assert!(!newer("v1.0.5-1", "1.0.8"));
    assert!(!newer("v1.0.8", "1.0.8"));
    assert!(!newer("nonsense", "1.0.8"));
}

#[test]
fn update_install_replaces_the_package_and_keeps_the_rest() {
    use crate::update::{install, STAGING_DIR};
    let here = std::env::temp_dir().join(format!("reskate-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&here);
    let package = here.join(STAGING_DIR).join("ReSkateServer-linux-x64");
    std::fs::create_dir_all(package.join("docs")).unwrap();
    std::fs::create_dir_all(here.join("docs")).unwrap();
    std::fs::create_dir_all(here.join("plugins")).unwrap();
    for (path, text) in [
        ("ReSkateServer", "old binary"),
        ("docs/plugins.md", "old docs"),
        ("ReSkateServer.json", "{\"name\": \"mine\"}"),
        ("plugins/info.lua", "-- mine"),
    ] {
        std::fs::write(here.join(path), text).unwrap();
    }
    for (path, text) in [
        ("ReSkateServer", "new binary"),
        ("libsteam_api.so", "lib"),
        ("docs/plugins.md", "new docs"),
        ("ReSkateServer.json", "{\"name\": \"from the archive\"}"),
    ] {
        std::fs::write(package.join(path), text).unwrap();
    }
    install(&package, &here).unwrap();
    let read = |path: &str| std::fs::read_to_string(here.join(path)).unwrap();
    assert_eq!(read("ReSkateServer"), "new binary");
    assert_eq!(read("libsteam_api.so"), "lib");
    assert_eq!(read("docs/plugins.md"), "new docs");
    assert_eq!(read("ReSkateServer.json"), "{\"name\": \"mine\"}");
    assert_eq!(read("plugins/info.lua"), "-- mine");
    assert!(!here.join(STAGING_DIR).exists());
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn logs_rotate_into_logs_and_keep_two_weeks() {
    use crate::{LogFile, LOG_DAYS};
    let folder = std::env::temp_dir().join(format!("reskate-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("logs")).unwrap();
    for day in 1..=20 {
        std::fs::write(folder.join(format!("logs/ReSkateServer-2026-09-{day:02}.log")), "old").unwrap();
    }
    std::fs::write(folder.join("ReSkateServer.log"), "yesterday\n").unwrap();
    let mut log = LogFile { file: None, date: "2026-10-05".into(), folder: folder.clone() };
    log.rotate("2026-10-04");
    assert_eq!(std::fs::read_to_string(folder.join("logs/ReSkateServer-2026-10-04.log")).unwrap(), "yesterday\n");
    assert!(log.file.is_some() && folder.join("ReSkateServer.log").exists());
    let kept = std::fs::read_dir(folder.join("logs")).unwrap().count();
    assert_eq!(kept, LOG_DAYS);
    assert!(!folder.join("logs/ReSkateServer-2026-09-01.log").exists());
    // A second rotation the same day adds to that day's file.
    std::fs::write(folder.join("ReSkateServer.log"), "after a restart\n").unwrap();
    log.rotate("2026-10-04");
    assert_eq!(std::fs::read_to_string(folder.join("logs/ReSkateServer-2026-10-04.log")).unwrap(), "yesterday\nafter a restart\n");
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn update_refuses_a_binary_that_does_not_run_here() {
    use crate::update::check_runs;
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("reskate-runs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let script = |name: &str, body: &str| {
        let path = folder.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let current = script("current", "echo 'ReSkateServer 1.0.9'");
    let older = script("older", "echo 'Unknown option --version.'; echo 'ReSkateServer [--config <file>]'; exit 1");
    let glibc = script(
        "glibc",
        "echo \"./ReSkateServer: /lib/x86_64-linux-gnu/libm.so.6: version 'GLIBC_2.44' not found (required by ./ReSkateServer)\" >&2; exit 1",
    );
    assert!(check_runs(&current).is_ok());
    assert!(check_runs(&older).is_ok());
    let error = check_runs(&glibc).unwrap_err();
    assert!(error.contains("GLIBC_2.44"), "{error}");
    assert!(check_runs(&folder.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn latest_release_tag_comes_from_the_redirect() {
    use crate::update::tag_from_location;
    let base = "https://github.com/wildesPepega/ReSkate-Linux-Server/releases/tag/";
    assert_eq!(tag_from_location(&format!("{base}v1.0.8-2")).as_deref(), Some("v1.0.8-2"));
    assert_eq!(tag_from_location(&format!("{base}v1.0.9?x=1")).as_deref(), Some("v1.0.9"));
    assert_eq!(tag_from_location("https://github.com/wildesPepega/ReSkate-Linux-Server/releases"), None);
    assert_eq!(tag_from_location(&format!("{base}latest-build")), None);
}

// Asks the real GitHub: cargo test --release -- --ignored quick_check_reaches_github
#[test]
#[ignore]
fn quick_check_reaches_github() {
    assert!(crate::update::quick_check().is_ok());
}

#[test]
fn console_lines_get_discord_categories() {
    use crate::discord::category;
    for (line, expected) in [
        ("pepZ.sh joined (76561198167564279, admin), 3/128 players, loaded in 7 s", "join"),
        ("oca left (Disconnected: Steam 1000, closed by the player's game, closed by player.)", "leave"),
        ("[EU]nein.live | Skate 3 is up on FullSkate3Map for 32 players.", "start"),
        ("Shutting down.", "stop"),
        ("Restarting for the update.", "stop"),
        ("ReSkate Linux Server 1.0.8-2 is available (this is 1.0.8-1); downloading it.", "update"),
        ("Installed ReSkate Linux Server 1.0.8-2; restarting.", "update"),
        ("ReSkate 1.0.9 is out; this Linux server is 1.0.8-3. It updates itself once a Linux build of it is released.", "update"),
        ("[throwdown] Sinful placed a Spot Battle drop near (265, 273, -521)", "throwdown"),
        ("[anticheat] mason418's game is running at 1.18x speed (a speed hack?).", "anticheat"),
        ("[chat] Bob: someone joined (76561198000000000), 1/2 players", "chat"),
        ("[party chat] Bob: hi", "party"),
        ("[command] consonant: /party invite a", "command"),
        ("[admin] pepZ.sh: kick Bob", "admin"),
        ("[vote] Bob started a vote to change the map.", "vote"),
        ("Everyone has loaded San Vansterdam.", "map"),
        ("[objects] Bob placed own_bkramps_generic_kickercurvedlarge_00001 at (-1264, 936, -877)", "objects"),
        ("[greeter] 4 players online", "plugins"),
        ("[plugins] 2 loaded: info (/discord)", "plugins"),
        ("World layers: 262 from world-layers.json.", "server"),
        ("ReSkate Linux Server 1.0.8-3", "server"),
    ] {
        assert_eq!(category(line), expected, "{line}");
    }
    assert!(crate::discord::valid_webhook("https://discord.com/api/webhooks/123/abc"));
    assert!(!crate::discord::valid_webhook("https://example.com/api/webhooks/123/abc"));
    assert!(!crate::discord::valid_webhook("http://discord.com/api/webhooks/123/abc"));
}

#[test]
fn discord_lines_reach_the_webhook_in_batches() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/webhooks/1/token", listener.local_addr().unwrap());
    let (sender, bodies) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            sender.send(String::from_utf8(body).unwrap()).unwrap();
            stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    });
    fn quiet(_: &str) {}
    let events = vec!["join".to_string(), "leave".to_string(), "stop".to_string()];
    assert!(crate::discord::start_checked(&url, &events, "fancy", "x", quiet, false).unwrap_err().contains("style"));
    let summary = crate::discord::start_checked(&url, &events, "embed", "Test *Server*", quiet, false).unwrap();
    assert!(summary.contains("join, leave, stop"));
    crate::discord::post("Bob_1 joined (76561198000000001), 1/16 players", "10:00:00");
    crate::discord::post("[chat] Bob_1: @everyone look", "10:00:01"); // not chosen
    crate::discord::post("Bob_1 left (Disconnected.)", "10:00:02");
    crate::discord::post("Shutting down.", "10:00:03");
    crate::discord::flush(std::time::Duration::from_secs(5));
    let body: serde_json::Value = serde_json::from_str(&bodies.recv_timeout(std::time::Duration::from_secs(5)).unwrap()).unwrap();
    assert_eq!(body["username"], "Test *Server*");
    assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
    let embeds = body["embeds"].as_array().unwrap();
    assert_eq!(embeds.len(), 3, "{body}");
    assert_eq!(embeds[0]["title"], "🟢 Player joined");
    assert_eq!(embeds[0]["description"], "Bob\\_1 joined (76561198000000001), 1/16 players");
    assert_eq!(embeds[0]["color"], 0x57F287);
    assert_eq!(embeds[0]["footer"]["text"], "Test *Server*");
    assert!(embeds[0]["timestamp"].as_str().unwrap().ends_with('Z'));
    assert_eq!(embeds[1]["title"], "🔴 Player left");
    assert_eq!(embeds[2]["title"], "⛔ Server stopped");
    // SUPPRESS_EMBEDS (flags 4) would hide every embed: the messages arrived empty.
    assert!(body.get("flags").is_none(), "{body}");
    assert!(!body.to_string().contains("everyone"));
}

#[test]
fn embed_time_stamps_are_iso_8601() {
    assert_eq!(crate::discord::iso8601(0), "1970-01-01T00:00:00Z");
    assert_eq!(crate::discord::iso8601(1_791_106_869), "2026-10-04T09:41:09Z");
    assert_eq!(crate::discord::iso8601(951_782_400), "2000-02-29T00:00:00Z");
}

#[test]
fn physics_extras_round_trip_and_limits() {
    let mut p = base(kind::PHYSICS_EXTRAS);
    p.extras = vec![1, 2, 3, 250];
    let raw = encode(&p, false);
    assert_eq!(decode(&raw).unwrap().extras, p.extras);
    // A length that does not match what follows, or more than the limit, is refused.
    let mut short = raw.clone();
    short.pop();
    assert!(decode(&short).is_none());
    p.extras = vec![0; MAX_PHYSICS_EXTRAS];
    assert_eq!(decode(&encode(&p, false)).unwrap().extras.len(), MAX_PHYSICS_EXTRAS);
}

#[test]
fn relay_budgets_hold_one_source_to_what_a_game_sends() {
    let mut outfit = OutfitBudget::default();
    for _ in 0..OUTFIT_BURST {
        assert!(outfit.accept(1_000_000));
    }
    assert!(!outfit.accept(2_000_000));
    assert!(outfit.accept(6_000_000));

    let mut sound = SoundBudget::default();
    assert!(sound.accept(1_000_000, 400));
    assert!(!sound.accept(1_100_000, 1));
    assert!(sound.accept(2_000_000, 4));
    let mut packets = SoundBudget::default();
    let mut accepted = 0;
    while packets.accept(1_000_000, 0) {
        accepted += 1;
    }
    assert_eq!(accepted, TICK_RATES[3] + 30);
}

#[test]
fn join_backoff_grows_and_is_forgotten() {
    let mut backoff = JoinBackoff::default();
    let start = 10_000_000;
    // The first failure may try again at once; the second waits 5 s, the third 10 s.
    assert_eq!(backoff.failed(PLAYER, start), 1);
    assert!(!backoff.waiting(PLAYER, start));
    assert_eq!(backoff.failed(PLAYER, start), 2);
    assert!(backoff.waiting(PLAYER, start + 4_999_999) && !backoff.waiting(PLAYER, start + 5_000_000));
    assert_eq!(backoff.failed(PLAYER, start), 3);
    assert!(backoff.waiting(PLAYER, start + 9_999_999) && !backoff.waiting(PLAYER, start + 10_000_000));
    assert!(!backoff.waiting(OTHER, start));
    // Never longer than ten minutes.
    for _ in 0..20 {
        backoff.failed(PLAYER, start);
    }
    assert!(backoff.waiting(PLAYER, start + 599_000_000) && !backoff.waiting(PLAYER, start + 600_000_000));
    // Joining clears it; half an hour without a failure forgets it.
    backoff.joined(PLAYER);
    assert!(!backoff.waiting(PLAYER, start));
    backoff.failed(OTHER, start);
    backoff.failed(OTHER, start);
    let later = start + 31 * 60 * 1_000_000;
    assert_eq!(backoff.failed(OTHER, later), 1);
    backoff.prune(later + 31 * 60 * 1_000_000);
    assert!(!backoff.waiting(OTHER, later));
}

#[test]
fn receive_budget_counts_by_arrival() {
    // A server held up for five seconds reads five seconds of normal traffic in one go: counted
    // by arrival, that is never a flood.
    let mut budget = ReceiveBudget::default();
    for second in 0..5u64 {
        for i in 0..100u64 {
            assert!(budget.accept(1_000_000 + second * 1_000_000 + i * 10_000, 100, 1));
        }
    }
    // A packet read after a later one (another lane) is counted in the current second.
    assert!(budget.accept(5_500_000, 100, 1));
    assert!(budget.accept(1_000_000, 100, 1));
}

#[test]
fn server_tags_keep_whole_characters() {
    let mut a = crate::steam::Advertisement {
        name: "Café, Bar".into(),
        map: "San Vanelona".into(),
        players: 3,
        max_players: 32,
        password: false,
        listed: true,
        secret: 0xabc,
    };
    let tags = String::from_utf8(crate::steam::server_tags(&a)).unwrap();
    assert!(tags.starts_with(&format!("reskate,v{PROTOCOL_VERSION},")), "{tags}");
    assert!(tags.ends_with(",nCafé  Bar"), "{tags}");
    // A name too long for the tags is cut, never through a character, and the list stays
    // under Steam's 128 byte limit.
    a.name = "é".repeat(100);
    let tags = crate::steam::server_tags(&a);
    assert!(tags.len() <= 127);
    let text = String::from_utf8(tags).unwrap();
    let name = &text[text.rfind(",n").unwrap() + 2..];
    assert!(!name.is_empty() && name.chars().all(|c| c == 'é'));
}

#[test]
fn status_page_answers_get_status_only() {
    use crate::status::response;
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
    let ok = text(response(b"GET /status HTTP/1.1\r\nHost: x\r\n\r\n", "{\"players\":3}"));
    assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"), "{ok}");
    assert!(ok.contains("Content-Type: application/json; charset=utf-8\r\n") && ok.contains("Content-Length: 13\r\n"));
    assert!(ok.contains("Access-Control-Allow-Origin: *\r\n") && ok.ends_with("\r\n\r\n{\"players\":3}"));
    assert!(text(response(b"GET /status.json?x=1 HTTP/1.1\r\n\r\n", "{}")).starts_with("HTTP/1.1 200"));
    let head = text(response(b"HEAD /status HTTP/1.1\r\n\r\n", "{\"players\":3}"));
    assert!(head.contains("Content-Length: 13\r\n") && head.ends_with("\r\n\r\n"));
    assert!(text(response(b"GET / HTTP/1.1\r\n\r\n", "{}")).starts_with("HTTP/1.1 404"));
    assert!(text(response(b"POST /status HTTP/1.1\r\n\r\n", "{}")).starts_with("HTTP/1.1 405"));
    assert!(text(response(b"", "{}")).starts_with("HTTP/1.1 405"));
    assert!(text(response(&[0xff, 0xfe, b'\n'], "{}")).starts_with("HTTP/1.1 405"));
}

#[test]
fn status_page_serves_the_latest_snapshot() {
    use std::io::{Read, Write};
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let server = crate::status::StatusServer::start(port).unwrap();
    server.set("{\"players\":7}".into());
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(b"GET /status HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200 OK") && reply.ends_with("{\"players\":7}"), "{reply}");
    // A client that never sends a full request is answered after the timeout, not held forever.
    let mut idle = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    idle.write_all(b"GET /sta").unwrap();
    let mut reply = String::new();
    idle.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
}

#[test]
fn status_json_shows_public_facts_only() {
    let mut config = ServerConfig { name: "Status test".into(), max_players: 10, ..Default::default() };
    config.password = "secret".into();
    let host = crate::host::Host::new(config, crate::steam::SteamTransport::new(), Box::new(|_: &str| {}));
    let status = host.status();
    assert_eq!(status["name"], "Status test");
    assert_eq!((status["players"].as_u64(), status["max_players"].as_u64()), (Some(0), Some(10)));
    assert_eq!((status["password"].as_bool(), status["uptime_seconds"].as_u64()), (Some(true), Some(0)));
    assert_eq!(status["protocol"].as_u64(), Some(u64::from(PROTOCOL_VERSION)));
    assert!(status["player_list"].as_array().is_some_and(|l| l.is_empty()));
    // Never the password itself, and no join code before the server has a Steam ID.
    assert!(!status.to_string().contains("secret") && status["join_code"].is_null());
}

#[test]
fn plugins_keep_data_and_hear_map_and_stop() {
    use crate::plugins::{PlayerInfo, PluginManager};
    let folder = std::env::temp_dir().join(format!("reskate-plugin-data-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy("examples/plugins/playtime.lua", folder.join("playtime.lua")).unwrap();
    std::fs::write(
        folder.join("store.lua"),
        r#"
reskate.command("store", function(p, args, line)
    if line == "save" then
        return tostring(reskate.save({ list = { 1, 2, 3 }, name = "x", nested = { deep = true }, [5] = "five", ratio = 0.5 }))
    elseif line == "bad" then
        local ok, err = pcall(reskate.save, { f = function() end })
        return tostring(ok) .. " " .. tostring(err)
    end
    local d = reskate.load()
    if d.list == nil then return "empty" end
    return table.concat({ #d.list, d.list[3], d.name, tostring(d.nested.deep), d["5"], d.ratio }, ",")
end)
reskate.on("map", function(map) reskate.log("map " .. map) end)
reskate.on("stop", function(reason) reskate.broadcast("bye: " .. reason) end)
"#,
    )
    .unwrap();
    let mut plugins = PluginManager::default();
    let all = plugins.load(&folder, 1_000_000_000).join("\n");
    assert!(all.contains("2 loaded"), "{all}");
    let snapshot = plugin_snapshot();
    let alice = snapshot.players[0].clone();
    let store = |plugins: &PluginManager, line: &str| plugins.command(plugin_snapshot(), &alice, "store", line).unwrap().1;
    assert_eq!(store(&plugins, "load"), "empty");
    assert_eq!(store(&plugins, "save"), "true");
    assert_eq!(store(&plugins, "load"), "3,3,x,true,five,0.5");
    assert!(store(&plugins, "bad").starts_with("false") && store(&plugins, "bad").contains("cannot be saved"));
    assert_eq!(store(&plugins, "load"), "3,3,x,true,five,0.5"); // a failed save keeps the old data

    // Map and stop events reach the handlers.
    assert_eq!(plugin_text(&plugins.map(snapshot.clone(), "Isle of Grom")), vec!["log: [store] map Isle of Grom"]);
    let stop = plugin_text(&plugins.stop(snapshot.clone(), "The server is shutting down."));
    assert!(stop.contains(&"all: bye: The server is shutting down.".to_string()), "{stop:?}");

    // Playtime: Alice joins at 0 s and is online 600 s; it is saved and survives a reload.
    let mut joining = alice.clone();
    joining.online_seconds = 0;
    plugins.join(snapshot.clone(), &joining);
    let (_, reply) = plugins.command(snapshot.clone(), &alice, "playtime", "").unwrap();
    assert_eq!(reply, "Alice: 10m");
    let mut later = plugin_snapshot();
    later.players[0].online_seconds = 3900;
    let (_, reply) = plugins.command(later.clone(), &alice, "top", "").unwrap();
    assert!(reply.starts_with("1. Alice 1h 05m"), "{reply}");
    plugins.stop(later.clone(), "restart");
    let saved = std::fs::read_to_string(folder.join("data").join("playtime.json")).unwrap();
    assert!(saved.contains("76561198000000001") && saved.contains("3900"), "{saved}");
    // Reloaded while Alice is still online: her time so far is not counted twice.
    let mut reloaded = PluginManager::default();
    reloaded.load(&folder, 2_000_000_000);
    let (_, reply) = reloaded.command(later.clone(), &alice, "playtime", "alice").unwrap();
    assert_eq!(reply, "Alice: 1h 05m");
    let leaving = PlayerInfo { online_seconds: 4500, ..alice.clone() };
    reloaded.leave(later, &leaving, "Disconnected.");
    let (_, reply) = reloaded.command(plugin_snapshot(), &alice, "playtime", "").unwrap();
    assert_eq!(reply, "Alice: 1h 15m");
    let _ = std::fs::remove_dir_all(&folder);
}

fn outfit_packet() -> Packet {
    let mut p = base(kind::COSMETICS);
    p.appearance = Appearance {
        skater: CosmeticRecipe {
            key: SKATER_RECIPE_KEY,
            version: 2,
            scalars: vec![0x3f80_0000],
            items: vec![CosmeticSlot { slot: 4, asset: b"Own_TopShirt".to_vec(), parameters: vec![1, 2] }],
        },
        board: CosmeticRecipe { key: BOARD_RECIPE_KEY, version: 1, scalars: vec![0x3f80_0000], items: vec![CosmeticSlot { slot: 13, asset: b"Own_Deck".to_vec(), parameters: vec![7] }] },
        card: PlayerCard { background: 1, emblem: 2, title: 3 },
        ..Default::default()
    };
    p
}

#[test]
fn outfits_carry_hidden_tags_and_mark_styles() {
    let p = outfit_packet();
    let bytes = encode(&p, false);
    let decoded = decode(&bytes).unwrap();
    assert!(decoded.appearance == p.appearance && !decoded.appearance.hide_tag && !decoded.appearance.hide_items);
    // A player's choices to go without their backend tag, or its animated items, each on its own.
    for (tag, items) in [(true, false), (false, true), (true, true)] {
        let mut hidden = p.clone();
        hidden.appearance.hide_tag = tag;
        hidden.appearance.hide_items = items;
        let told = decode(&encode(&hidden, false)).unwrap();
        assert!(told.appearance.hide_tag == tag && told.appearance.hide_items == items && told.appearance == hidden.appearance);
        assert!(told.appearance != p.appearance);
    }
    // How each marked cosmetic animates.
    let mut styled = p.clone();
    styled.appearance.marks[0] = MarkStyle { mode: 2, from: [1, 2, 3], to: [250, 251, 252], speed: 2 };
    styled.appearance.marks[4] = MarkStyle { mode: 1, from: [0; 3], to: [0; 3], speed: 1 };
    styled.appearance.marks[MARK_ITEMS - 1] = MarkStyle { mode: 3, from: [9, 8, 7], to: [0; 3], speed: 0 };
    let raw = encode(&styled, false);
    let kept = decode(&raw).unwrap();
    assert!(kept.appearance == styled.appearance && kept.appearance.marks[0].to[2] == 252 && kept.appearance.marks[1] == MarkStyle::default());
    assert!(!valid_mark_style(&MarkStyle { mode: 4, ..Default::default() }) && !valid_mark_style(&MarkStyle { speed: 3, ..Default::default() }));
    // The flags and styles are the last 1 + 12 * 8 bytes: a flag or style no menu can make is refused.
    let flags_at = raw.len() - 1 - MARK_ITEMS * 8;
    let mut corrupt = raw.clone();
    corrupt[flags_at] = 4;
    assert!(decode(&corrupt).is_none());
    let mut corrupt = raw.clone();
    corrupt[flags_at + 1] = 4; // the first style's mode
    assert!(decode(&corrupt).is_none());
    let mut corrupt = raw.clone();
    corrupt[flags_at + 8] = 3; // the first style's speed
    assert!(decode(&corrupt).is_none());
    for n in 0..raw.len() {
        assert!(decode(&raw[..n]).is_none(), "truncated outfit accepted at {n}");
    }
    // The server relays it through its delta codec unchanged.
    let sender = crate::wire::DeltaSender::default();
    let mut receiver = crate::wire::DeltaReceiver::default();
    let mut missing = false;
    let update = sender.prepare(&styled);
    assert_eq!(receiver.receive(&update.bytes, &mut missing, 1).unwrap().appearance, styled.appearance);
}

#[test]
fn global_ban_lists_are_read_strictly() {
    use crate::global_bans::parse_ban_list;
    const GRIEFER: u64 = 76561198000000009;
    const CHEATER: u64 = 76561198000000010;
    // A list with nobody on it is still a list.
    assert_eq!(parse_ban_list(r#"{"categories":{"dev":["76561198000000011"]},"banned":[]}"#), Ok(vec![]));
    assert_eq!(
        parse_ban_list(r#"{"categories":{},"banned":["76561198000000010","76561198000000009","76561198000000010"]}"#),
        Ok(vec![GRIEFER, CHEATER])
    );
    // A backend from before it had bans bans nobody; a category this build does not know is ignored.
    assert_eq!(parse_ban_list(r#"{"categories":{"dev":[],"someday":[1]}}"#), Ok(vec![]));
    // An answer that is not the lists (the backend down, a proxy's error page) is refused, so it
    // lifts no ban.
    for wrong in [
        "",
        "<html>502 Bad Gateway</html>",
        r#"{"banned":["76561198000000009"]}"#,
        r#"{"categories":{},"banned":["everyone"]}"#,
        r#"{"categories":{},"banned":"76561198000000009"}"#,
        r#"{"categories":{},"banned":[76561198000000009]}"#,
        r#"{"categories":{},"banned":["76561197960265728"]}"#,
        r#"{"categories":{},"banned":["076561198000000009"]}"#,
        r#"{"categories":{"dev":["nobody"]},"banned":[]}"#,
    ] {
        assert!(parse_ban_list(wrong).is_err(), "accepted: {wrong}");
    }
}

#[test]
fn globally_banned_players_are_turned_away_unless_the_server_opts_out() {
    const GRIEFER: u64 = 76561198000000009;
    let mut host = crate::host::Host::new(ServerConfig::default(), crate::steam::SteamTransport::new(), Box::new(|_: &str| {}));
    assert!(!host.globally_banned(GRIEFER), "a player was banned before any list was read");
    host.set_global_bans(vec![GRIEFER]);
    assert!(host.globally_banned(GRIEFER) && !host.globally_banned(PLAYER));
    host.config.global_bans = false;
    assert!(!host.globally_banned(GRIEFER));
}

// The deployed backend, read the way the server reads it. Off the network in normal test runs:
// cargo test -- --ignored global_ban_list_reaches_the_backend
#[test]
#[ignore]
fn global_ban_list_reaches_the_backend() {
    let (ids, tokens_required) = crate::global_bans::read_ban_list().unwrap();
    println!("{} banned, steam_token required: {tokens_required}", ids.len());
}

#[test]
fn protocol_42_carries_map_names_pools_and_rotation() {
    let destination = "Levels/Game/DingoLevel_Root/DingoLevel_Root|Levels/Game/dingolevel_reskate_momentumpark/x";
    for k in [kind::WORLD_STATE, kind::MAP_OFFER] {
        let mut p = base(k);
        p.map = map_hash(destination);
        p.destination = destination.into();
        p.map_label = "Momentum Park".into();
        let back = decode(&encode(&p, false)).unwrap();
        assert_eq!((back.map_label.as_str(), back.destination.as_str()), ("Momentum Park", destination));
        // An unnamed map is fine; an overlong name is refused.
        p.map_label.clear();
        assert!(decode(&encode(&p, false)).unwrap().map_label.is_empty());
        p.map_label = "a".repeat(MAX_MEMBER_NAME + 1);
        assert!(std::panic::catch_unwind(|| encode(&p, false)).is_err());
    }
    // An empty world state (between maps) still carries its (empty) name.
    let mut between = base(kind::WORLD_STATE);
    between.map = 0;
    assert!(decode(&encode(&between, false)).is_some());

    let mut maps = base(kind::MAPS);
    maps.maps = vec!["Levels/Game/BAM_LevelRoot/BAM_LevelRoot".into(), "Levels/Custom/bbcity/bbcity".into()];
    maps.map_pool = vec![1, 0];
    maps.map_rotation = 20;
    let back = decode(&encode(&maps, false)).unwrap();
    assert_eq!((back.map_pool, back.map_rotation), (vec![1, 0], 20));
    for (pool, rotation) in [(vec![2], 0), (vec![0, 0], 0), (vec![], MAX_MAP_ROTATION as u16 + 1)] {
        let mut bad = maps.clone();
        bad.map_pool = pool;
        bad.map_rotation = rotation;
        assert!(std::panic::catch_unwind(|| encode(&bad, false)).is_err());
    }
    // A pool past the map list on the wire is refused too (the count sits after the assets).
    let mut raw = encode(&maps, false);
    let pool_count_at = raw.len() - 2 - 2 * 2 - 2;
    raw[pool_count_at] = 3;
    assert!(decode(&raw).is_none());
    assert!(valid_map_pool(&[], 0) && valid_map_pool(&[0, 2, 1], 3) && !valid_map_pool(&[1, 1], 3) && !valid_map_pool(&[3], 3));
}

#[test]
fn direct_messages_keep_their_marker() {
    use crate::text::dm_line;
    assert_eq!(dm_line("Server", "", "hi", 100), "[DM from Server] hi");
    assert_eq!(dm_line("Player", "party", "hi", 100), "[DM from Player to party] hi");
    assert_eq!(dm_line("Player", "admins", "hi", 100), "[DM from Player to admins] hi");
    let cut = dm_line("Server", "", &"a".repeat(500), 40);
    assert!(cut.len() == 40 && cut.starts_with("[DM from Server] "));
    // Never cut through a UTF-8 character.
    assert_eq!(dm_line("S", "", "ééé", 15), "[DM from S] é");
    assert_eq!(dm_line("Server", "", "hi", 3), "[DM from Server] ");
}

// The same maps the config test loads (custom map bbcity), so the shared map list is the same
// whichever test loads it.
fn load_test_levels(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("reskate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mods = folder.join("Mods").join("bbcity");
    std::fs::create_dir_all(&mods).unwrap();
    std::fs::write(mods.join("reskate-levels.json"), r#"{"levels":[{"asset":"Levels/Custom/BBCity/BBCity","displayName":"bbcity"}]}"#).unwrap();
    assert!(load_levels(&folder.join("Mods")).is_empty());
    folder
}

#[test]
fn map_pool_and_rotation_order() {
    use crate::config::{in_map_pool, levels, next_pool_map, pool_levels};
    let folder = load_test_levels("pool");
    let mut pool = ServerConfig::default();
    assert!(pool_levels(&pool).len() == levels().len() && in_map_pool(&pool, "Stadium 2"));
    pool.map_pool = vec!["Isle".into(), "Isle of Grom".into(), "San Vansterdam".into(), "Stadium 1".into()];
    assert_eq!(pool_levels(&pool).len(), 3);
    assert!(in_map_pool(&pool, "San Vansterdam"));
    assert!(in_map_pool(&pool, "Levels/Game/DingoLevel_SDM/DingoLevel_SDM_Int_001/DingoLevel_SDM_Int_001"));
    assert!(!in_map_pool(&pool, "Super Ultra Mega Resort") && !in_map_pool(&pool, "Nowhere"));
    let next = |pool: &ServerConfig, map: &str| next_pool_map(pool, map).map(|l| l.name).unwrap_or_default();
    assert_eq!(next(&pool, "Isle of Grom"), "San Vansterdam");
    assert_eq!(next(&pool, "Stadium 1"), "Isle of Grom");
    assert_eq!(next(&pool, "Super Ultra Mega Resort"), "Isle of Grom");
    pool.map_pool = vec!["Isle of Grom".into()];
    assert!(next(&pool, "Isle of Grom").is_empty() && next(&pool, "San Vansterdam") == "Isle of Grom");
    assert_eq!(config_error(&pool), "");
    pool.map_pool = vec!["Isle of Grom".into(), "Nowhere".into()];
    assert!(!config_error(&pool).is_empty());

    // Saved and read back; bad entries dropped, the rotation capped.
    let file = folder.join("ReSkateServer.json");
    pool.map_pool = vec!["Isle of Grom".into(), "Stadium 1".into()];
    pool.map_rotation = 15;
    pool.file = file.clone();
    save_config(&pool).unwrap();
    let mut added = Vec::new();
    let back = load_config(&file, &mut added).unwrap();
    assert_eq!((back.map_pool.clone(), back.map_rotation), (pool.map_pool.clone(), 15));
    std::fs::write(&file, r#"{"map_pool": ["Isle of Grom", 7, ""], "map_rotation_minutes": 5000}"#).unwrap();
    let odd = load_config(&file, &mut added).unwrap();
    assert_eq!((odd.map_pool, odd.map_rotation), (vec!["Isle of Grom".to_string()], 1440));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn map_pool_and_rotation_commands() {
    let folder = load_test_levels("pool-commands");
    let config = ServerConfig { file: folder.join("ReSkateServer.json"), ..Default::default() };
    let mut host = crate::host::Host::new(config, crate::steam::SteamTransport::new(), Box::new(|_: &str| {}));
    assert_eq!(host.command("map-pool", 0), "Map pool: every map");
    assert!(host.command("map-pool add Isle of Grom", 0).contains("already in the map pool"));
    // Removing from "every map" keeps all the others.
    assert!(host.command("map-pool remove Stadium 2", 0).contains("removed from"));
    assert!(!host.config.map_pool.iter().any(|m| m == "Stadium 2") && host.config.map_pool.len() >= 2);
    assert!(host.command("map-pool clear", 0).contains("cleared"));
    assert!(host.config.map_pool.is_empty());
    host.config.map_pool = vec!["Isle of Grom".into()];
    assert!(host.command("map-pool remove Isle of Grom", 0).contains("needs at least one map"));
    assert!(host.command("map-pool add Nowhere", 0).starts_with("No single map"));
    assert_eq!(host.command("rotation", 0), "Map rotation is off.");
    assert!(host.command("rotation 2000", 0).starts_with("rotation <1-1440"));
    assert!(host.command("rotation 30", 0).starts_with("The map changes every 30 min"));
    assert_eq!(host.config.map_rotation, 30);
    assert_eq!(host.command("rotation off", 0), "Map rotation is off.");
    // Direct messages need someone to send them to.
    assert!(host.command("msg-admins hello", 0).contains("No admins are online"));
    assert!(host.command("msg Bob hi", 0).starts_with("No single connected player"));
    assert_eq!(host.command("msg Bob", 0), "msg <player> <text>");
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn server_names_follow_the_browser_rule() {
    assert!(valid_server_name("Old Server") && valid_server_name("[EU] Skate_Park-2 (24x7)") && valid_server_name("a"));
    // ReSkate 1.1.4: "/" too.
    assert!(valid_server_name("EU/West 24/7") && SERVER_NAME_RULE.contains("- _ / [ ] ( )"));
    for bad in ["", &"a".repeat(65), "Best! Server", "café", "a.b", "<b>x</b>", " padded", "padded ", "[]--()", "two\nlines", "///", "a\\b"] {
        assert!(!valid_server_name(bad), "accepted: {bad:?}");
    }
    let config = ServerConfig { name: "Best! Server".into(), ..Default::default() };
    let mut host = crate::host::Host::new(config, crate::steam::SteamTransport::new(), Box::new(|_: &str| {}));
    // A name from before the rule still runs, but is not listed.
    assert_eq!(host.status()["listed"], false);
    assert!(host.command("name Still! Bad", 0).starts_with("Server names are 1 to 64 letters"));
    assert_eq!(host.config.name, "Best! Server");
    host.config.file = std::env::temp_dir().join(format!("reskate-name-{}.json", std::process::id()));
    assert!(host.command("name [EU] Good Server", 0).starts_with("Server renamed to [EU] Good Server"));
    assert_eq!(host.status()["listed"], true);
    let _ = std::fs::remove_file(&host.config.file);
}

#[test]
fn global_ban_lists_check_the_centrix_category() {
    use crate::global_bans::parse_ban_list;
    assert_eq!(parse_ban_list(r#"{"categories":{"centrix":["76561198000000011"]},"banned":[]}"#), Ok(vec![]));
    assert!(parse_ban_list(r#"{"categories":{"centrix":["nobody"]},"banned":[]}"#).is_err());
}

#[test]
fn thunderstore_links() {
    use crate::thunderstore::{parse_package, split_links, Package};
    let latest = Some(Package { namespace: "Dingo".into(), name: "BB_City".into(), version: None });
    let pinned = Some(Package { namespace: "Dingo".into(), name: "BB_City".into(), version: Some("1.2.3".into()) });
    assert_eq!(parse_package("https://thunderstore.io/c/skate/p/Dingo/BB_City/"), latest);
    assert_eq!(parse_package(" https://thunderstore.io/c/skate/p/Dingo/BB_City?tab=readme "), latest);
    assert_eq!(parse_package("https://thunderstore.io/c/skate/p/Dingo/BB_City/v/1.2.3/"), pinned);
    assert_eq!(parse_package("https://thunderstore.io/package/Dingo/BB_City/"), latest);
    assert_eq!(parse_package("https://skate.thunderstore.io/package/Dingo/BB_City/"), latest);
    assert_eq!(parse_package("https://thunderstore.io/package/download/Dingo/BB_City/1.2.3/"), pinned);
    assert_eq!(parse_package("Dingo-BB_City"), latest);
    assert_eq!(parse_package("Dingo-BB_City-1.2.3"), pinned);
    for bad in ["https://example.com/c/skate/p/Dingo/BB_City/", "https://thunderstore.io/c/skate/", "Dingo", "Dingo-BB City", "Dingo-BB_City-1.2", "../x-y"] {
        assert_eq!(parse_package(bad), None, "{bad}");
    }
    assert_eq!(split_links("a-b, c-d;e-f\n g-h"), ["a-b", "c-d", "e-f", "g-h"]);
    assert_eq!(latest.unwrap().folder(), "Dingo-BB_City");
}

// A zip as Thunderstore serves one: stored and deflated files, the manifest in a subfolder.
fn test_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let (mut zip, mut directory) = (Vec::new(), Vec::new());
    for (name, data, deflate) in files {
        let packed = if *deflate {
            let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method: u16 = if *deflate { 8 } else { 0 };
        let offset = zip.len() as u32;
        zip.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0]);
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&[0; 8]); // time, date, crc
        zip.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(&packed);
        directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0]);
        directory.extend_from_slice(&method.to_le_bytes());
        directory.extend_from_slice(&[0; 8]);
        directory.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = zip.len() as u32;
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    zip.extend_from_slice(&start.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);
    zip
}

#[test]
fn thunderstore_zip_manifest() {
    use crate::thunderstore::manifests_in_zip;
    let one = br#"{"levels":[{"asset":"Levels/Custom/BBCity/BBCity","displayName":"bbcity"}]}"#;
    let two = "\u{feff}{\"levels\":[{\"asset\":\"Levels/Custom/Two/Two\"}]}".as_bytes();
    let big = vec![7u8; 200_000]; // the map itself, never read
    let zip = test_zip(&[
        ("manifest.json", b"{}", false),
        ("BepInEx/plugins/BBCity/BBCity.pak", &big, true),
        ("BepInEx/plugins/BBCity/reskate-levels.json", one, true),
        ("Other/RESKATE-LEVELS.JSON", two, false),
    ]);
    let manifest = manifests_in_zip(zip).unwrap();
    let assets: Vec<&str> = manifest["levels"].as_array().unwrap().iter().map(|l| l["asset"].as_str().unwrap()).collect();
    assert_eq!(assets, ["Levels/Custom/BBCity/BBCity", "Levels/Custom/Two/Two"]);
    assert!(manifests_in_zip(test_zip(&[("manifest.json", b"{}", true)])).unwrap_err().contains("not a ReSkate map"));
    assert!(manifests_in_zip(b"not a zip".to_vec()).is_err());
    assert!(manifests_in_zip(test_zip(&[("reskate-levels.json", b"{\"maps\":[]}", false)])).is_err());
}

#[test]
fn thunderstore_removes_unlisted_packages_only() {
    let mods = std::env::temp_dir().join(format!("reskate-mapmods-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&mods);
    std::fs::create_dir_all(mods.join("Dingo-Old")).unwrap();
    std::fs::write(mods.join("Dingo-Old").join(crate::thunderstore::MARKER), "{}").unwrap();
    std::fs::create_dir_all(mods.join("ByHand")).unwrap();
    let report = crate::thunderstore::sync(&mods, &["not a link".into()]);
    assert!(!mods.join("Dingo-Old").exists() && mods.join("ByHand").exists());
    assert_eq!(report.lines.len(), 2, "{:?}", report.lines);
    assert!(report.new_maps.is_empty());
    let _ = std::fs::remove_dir_all(&mods);
}

#[test]
fn map_mods_setting() {
    let file = std::env::temp_dir().join(format!("reskate-mapmods-{}.json", std::process::id()));
    std::fs::write(&file, r#"{"map_mods": [" https://thunderstore.io/c/skate/p/Dingo/BB_City/ ", ""]}"#).unwrap();
    let config = load_config(&file, &mut Vec::new()).unwrap();
    assert_eq!(config.map_mods, ["https://thunderstore.io/c/skate/p/Dingo/BB_City/"]);
    save_config(&config).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"map_mods\""));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn map_mods_command() {
    use crate::thunderstore::{merge_links, MARKER};
    assert_eq!(
        merge_links(&["Dingo-A".into(), "https://thunderstore.io/c/skate/p/Dingo/B/".into()], &["Dingo-B-1.0.0".into(), "Dingo-C".into()]),
        ["Dingo-A", "https://thunderstore.io/c/skate/p/Dingo/B/", "Dingo-C"]
    );
    let folder = std::env::temp_dir().join(format!("reskate-mapmods-cmd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mods = folder.join("Mods");
    for name in ["Dingo-Old", "Dingo-Panel"] {
        std::fs::create_dir_all(mods.join(name)).unwrap();
        std::fs::write(mods.join(name).join(MARKER), r#"{"version":"1.0.0"}"#).unwrap();
        std::fs::write(mods.join(name).join("reskate-levels.json"), r#"{"levels":[{"asset":"Levels/Custom/Old/Old","displayName":"oldmap"}]}"#).unwrap();
    }
    // Reading the maps again (as the install does) must leave the shared list as the other tests
    // load it: with bbcity.
    std::fs::create_dir_all(mods.join("bbcity")).unwrap();
    std::fs::write(mods.join("bbcity").join("reskate-levels.json"), r#"{"levels":[{"asset":"Levels/Custom/BBCity/BBCity","displayName":"bbcity"}]}"#).unwrap();
    let mut config = ServerConfig::default();
    config.file = folder.join("ReSkateServer.json");
    config.map_mods = vec!["Dingo-Old".into()];
    let mut host = crate::host::Host::new(config, crate::steam::SteamTransport::new(), Box::new(|_: &str| {}));
    host.mods_dir = mods.clone();
    host.panel_map_mods = vec!["Dingo-Panel".into()];
    let list = host.command("map-mods", 0);
    assert!(list.contains("Dingo-Old 1.0.0: oldmap") && list.contains("Dingo-Panel 1.0.0: oldmap  (panel)"), "{list}");
    assert!(host.command("map-mods add not a link", 0).starts_with("map-mods add <"));
    assert!(host.command("map-mods add Dingo-Old-2.0.0", 0).contains("already listed"));
    assert!(host.command("map-mods remove Dingo-Panel", 0).contains("panel"));
    assert!(host.command("map-mods remove nothing", 0).starts_with("No map mod matches"));
    // The server's own map cannot go; pool entries can.
    host.config.map = "oldmap".into();
    assert!(host.command("map-mods remove oldmap", 0).contains("change the map first"));
    host.config.map = "San Vansterdam".into();
    host.config.map_pool = vec!["San Vansterdam".into(), "oldmap".into()];
    // Removing needs no network: the folder goes, the panel's too once it is off the list.
    host.panel_map_mods.clear();
    assert_eq!(host.command("map-mods remove oldmap", 0), "Removing Dingo-Old...");
    assert!(host.command("map-mods update", 0).contains("being installed"));
    for _ in 0..500 {
        host.finish_map_mods();
        if host.map_mods_job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(host.map_mods_job.is_none() && !mods.join("Dingo-Old").exists() && !mods.join("Dingo-Panel").exists());
    assert!(host.config.map_mods.is_empty());
    assert_eq!(host.config.map_pool, ["San Vansterdam"]);
    let mut config = ServerConfig::default();
    config.map = "OldMap".into();
    assert_eq!(crate::config::drop_removed_maps(&mut config, &["oldmap".into()]).len(), 1);
    assert_eq!(config.map, "San Vansterdam");
    assert!(std::fs::read_to_string(folder.join("ReSkateServer.json")).unwrap().contains("\"map_mods\": []"));
    let _ = std::fs::remove_dir_all(&folder);
}

// ReSkate 1.1.4 (Server/Test/server_config_tests.cpp, developer_identity_tests.cpp).
#[test]
fn steam_token_and_server_lists() {
    use crate::config::valid_steam_token;
    use crate::global_bans::{parse_ban_list, parse_tokens_required};
    // steam_token: empty (anonymous) unless set, kept on a rewrite, and only letters and digits.
    let file = std::env::temp_dir().join(format!("reskate-token-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut added = Vec::new();
    let mut config = load_config(&file, &mut added).unwrap();
    assert!(config.steam_token.is_empty() && std::fs::read_to_string(&file).unwrap().contains("\"steam_token\": \"\""));
    config.steam_token = "0123456789ABCDEF0123456789ABCDEF".into();
    save_config(&config).unwrap();
    assert_eq!(load_config(&file, &mut Vec::new()).unwrap().steam_token, config.steam_token);
    assert!(valid_steam_token("") && valid_steam_token(&config.steam_token));
    assert!(!valid_steam_token("not a token") && !valid_steam_token(&"A".repeat(65)));
    config.steam_token = "not a token".into();
    assert!(config_error(&config).contains("steam_token"));
    let _ = std::fs::remove_file(&file);

    // Servers: anonymous (type 4) or with a login token (type 3, the same ID every start).
    const TOKEN_SERVER: u64 = (1 << 56) | (3 << 52) | 12345;
    assert!(persistent_server_steam_id(TOKEN_SERVER) && !persistent_server_steam_id(SERVER));
    assert!(game_server_steam_id(SERVER) && !persistent_server_steam_id(PLAYER));

    // The backend's answer: the staff category, official and blocked servers, the token rule.
    let answer = format!(
        r#"{{"categories":{{"staff":["76561198000000011"]}},"banned":["76561198000000009"],"official_servers":["{TOKEN_SERVER}"],"blocked_servers":["{SERVER}"],"server_tokens_required":true}}"#
    );
    assert_eq!(parse_ban_list(&answer), Ok(vec![76561198000000009]));
    assert!(parse_tokens_required(&answer));
    assert!(!parse_tokens_required(r#"{"categories":{},"banned":[]}"#) && !parse_tokens_required("not json"));
    assert!(!parse_tokens_required(r#"{"categories":{},"server_tokens_required":"yes"}"#));
    // An official server must be one with a login token; any list that is not one refuses the answer.
    for wrong in [
        format!(r#"{{"categories":{{}},"official_servers":["{SERVER}"]}}"#),
        r#"{"categories":{},"blocked_servers":["76561198000000009"]}"#.to_string(),
        r#"{"categories":{"staff":["nobody"]}}"#.to_string(),
        r#"{"categories":{},"official_servers":"x"}"#.to_string(),
    ] {
        assert!(parse_ban_list(&wrong).is_err(), "accepted: {wrong}");
    }
}
