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
    assert_eq!(u16::from_le_bytes([raw[4], raw[5]]), 38);
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
    p.appearance = Appearance { skater: recipe(SKATER_RECIPE_KEY, 2), board: recipe(BOARD_RECIPE_KEY, 1), card: PlayerCard { background: 1, emblem: 2, title: 3 } };
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
            PlayerInfo { id: 76561198000000001, name: "Alice".into(), admin: true },
            PlayerInfo { id: 76561198000000002, name: "Bob".into(), admin: false },
        ],
        server: "Test Server".into(),
        map: "San Vansterdam".into(),
        max_players: 16,
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
    let summary = crate::discord::start_checked(&url, &events, "Test *Server*", quiet, false).unwrap();
    assert!(summary.contains("join, leave, stop"));
    crate::discord::post("Bob_1 joined (76561198000000001), 1/16 players", "10:00:00");
    crate::discord::post("[chat] Bob_1: @everyone look", "10:00:01"); // not chosen
    crate::discord::post("Bob_1 left (Disconnected.)", "10:00:02");
    crate::discord::post("Shutting down.", "10:00:03");
    crate::discord::flush(std::time::Duration::from_secs(5));
    let body: serde_json::Value = serde_json::from_str(&bodies.recv_timeout(std::time::Duration::from_secs(5)).unwrap()).unwrap();
    let content = body["content"].as_str().unwrap();
    assert_eq!(body["username"], "Test *Server*");
    assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
    assert_eq!(content.lines().count(), 3, "{content}");
    assert!(content.starts_with("🟢 `10:00:00` Bob\\_1 joined (76561198000000001), 1/16 players\n"), "{content}");
    assert!(content.contains("🔴 `10:00:02` Bob\\_1 left (Disconnected.)") && content.contains("⛔ `10:00:03` Shutting down."));
    assert!(!content.contains("everyone"));
}
