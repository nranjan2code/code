#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_flow::{ParseError, parse_flow};

#[test]
fn valid_flow_parses_with_layers() {
    let toml = r#"
[flow]
name = "demo"

[[nodes]]
id = "a"
type = "bash"
command = "echo a"

[[nodes]]
id = "b"
type = "agent"
prompt = "use {{a}}"

[[nodes]]
id = "c"
type = "merge"
deps = ["b"]
"#;
    let flow = parse_flow(toml).unwrap();
    assert_eq!(flow.nodes.len(), 3);
    let layers = vak_flow::parse::layers(&flow).unwrap();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0], vec!["a"]);
    assert_eq!(layers[1], vec!["b"]);
    assert_eq!(layers[2], vec!["c"]);
}

#[test]
fn independent_nodes_share_a_layer() {
    let toml = r#"
[flow]
name = "fan"

[[nodes]]
id = "x"
type = "bash"
command = "echo x"

[[nodes]]
id = "y"
type = "bash"
command = "echo y"

[[nodes]]
id = "z"
type = "merge"
deps = ["x", "y"]
"#;
    let layers = vak_flow::parse::layers(&parse_flow(toml).unwrap()).unwrap();
    assert_eq!(layers.len(), 2);
    assert!(layers[0].contains(&"x".to_string()));
    assert!(layers[0].contains(&"y".to_string()));
}

#[test]
fn cycle_is_rejected() {
    let toml = r#"
[flow]
name = "cycle"

[[nodes]]
id = "a"
type = "bash"
command = "echo a"
deps = ["b"]

[[nodes]]
id = "b"
type = "bash"
command = "echo b"
deps = ["a"]
"#;
    match parse_flow(toml) {
        Err(ParseError::Cycle(stuck)) => assert!(stuck.contains("a") && stuck.contains("b")),
        other => panic!("expected cycle, got {other:?}"),
    }
}

#[test]
fn unknown_dep_and_self_dep_rejected() {
    let toml = r#"
[flow]
name = "bad"

[[nodes]]
id = "a"
type = "bash"
command = "echo a"
deps = ["ghost"]
"#;
    assert!(matches!(
        parse_flow(toml),
        Err(ParseError::UnknownDep(node, dep)) if node == "a" && dep == "ghost"
    ));

    let toml = r#"
[flow]
name = "bad"

[[nodes]]
id = "a"
type = "bash"
command = "echo a"
deps = ["a"]
"#;
    assert!(matches!(parse_flow(toml), Err(ParseError::SelfDep(_))));
}

#[test]
fn unknown_type_and_missing_fields_rejected() {
    let toml = r#"
[flow]
name = "bad"

[[nodes]]
id = "a"
type = "quantum"
command = "echo a"
"#;
    assert!(matches!(
        parse_flow(toml),
        Err(ParseError::UnknownType(_, t)) if t == "quantum"
    ));

    let toml = r#"
[flow]
name = "bad"

[[nodes]]
id = "a"
type = "agent"
"#;
    assert!(matches!(
        parse_flow(toml),
        Err(ParseError::MissingField(id, _, field)) if id == "a" && field == "prompt"
    ));
}

#[test]
fn duplicate_ids_rejected() {
    let toml = r#"
[flow]
name = "dup"

[[nodes]]
id = "a"
type = "bash"
command = "echo 1"

[[nodes]]
id = "a"
type = "bash"
command = "echo 2"
"#;
    assert!(matches!(parse_flow(toml), Err(ParseError::Duplicate(d)) if d == "a"));
}
