//! Deterministic, parameterized scenarios for long-running generic-agent evals.
//!
//! A scenario is generated on demand from `(seed, index)`, so a large corpus
//! does not need to be stored in memory or checked into the repository. The
//! families deliberately cover research, data, writing, conversion, and
//! scheduling. The scripted provider makes this a runtime/tool-contract test,
//! not a benchmark of model reasoning.

use crate::runner::{EvalCase, ScriptedTurn};

const FAMILIES: [&str; 5] = ["data", "research", "writing", "conversion", "schedule"];

/// Build the stable scenario at `index` for `seed` without allocating any
/// neighboring scenarios. This lets `vak eval --generated N --offset K`
/// process arbitrarily large corpora in bounded batches.
pub fn generated_scenario(seed: u64, index: u64) -> EvalCase {
    let family = (index % FAMILIES.len() as u64) as usize;
    let variant = index / FAMILIES.len() as u64;
    let entropy = mix(seed ^ variant.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let id = format!("generated-{}-{seed:016x}-{index:012}", FAMILIES[family]);
    match family {
        0 => data_case(id, entropy, index),
        1 => research_case(id, entropy, index),
        2 => writing_case(id, entropy, index),
        3 => conversion_case(id, entropy, index),
        _ => schedule_case(id, entropy, index),
    }
}

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn base(
    id: String,
    description: &str,
    prompt: String,
    script: Vec<ScriptedTurn>,
    verify: String,
) -> EvalCase {
    EvalCase {
        id,
        description: description.into(),
        files: Vec::new(),
        prompt,
        script,
        outcome: None,
        verify,
    }
}

fn data_case(id: String, entropy: u64, index: u64) -> EvalCase {
    let count = 3 + (entropy % 6) as usize;
    let mut csv = String::from("item,amount\n");
    let mut total = 0_i64;
    for row in 0..count {
        let amount = (mix(entropy.wrapping_add(row as u64)) % 2_001) as i64 - 1_000;
        total += amount;
        csv.push_str(&format!("item-{row},{amount}\n"));
    }
    let mut case = base(
        id,
        "analyze a generated expense table and report its exact total",
        format!(
            "Read amounts-{index}.csv, calculate the total of the amount column, and write total.txt with only the number."
        ),
        vec![
            ScriptedTurn::tool(
                "bash",
                serde_json::json!({
                    "command": format!("awk -F, 'NR>1 {{s+=$2}} END {{printf \"%.0f\\n\", s}}' amounts-{index}.csv")
                }),
            ),
            ScriptedTurn::tool(
                "write",
                serde_json::json!({
                    "path": "total.txt", "content": format!("{total}\n")
                }),
            ),
            ScriptedTurn::Text("The total is recorded in total.txt.".into()),
        ],
        format!("test \"$(cat total.txt)\" = \"{total}\""),
    );
    case.files.push((format!("amounts-{index}.csv"), csv));
    case
}

fn research_case(id: String, entropy: u64, index: u64) -> EvalCase {
    const TOPICS: [&str; 6] = ["water", "transit", "forests", "housing", "energy", "health"];
    let topic = TOPICS[(entropy as usize) % TOPICS.len()];
    let first = 10 + (mix(entropy) % 90);
    let second = 10 + (mix(entropy ^ 0xa5a5) % 90);
    let source_a = format!("Source A: The {topic} program reached {first} communities in 2025.\n");
    let source_b =
        format!("Source B: The {topic} program reduced annual costs by {second} percent.\n");
    let report = format!(
        "# {topic} findings\n\nThe program reached {first} communities in 2025 [1]. Annual costs fell by {second} percent [2].\n\n## References\n- [1] Source A\n- [2] Source B\n"
    );
    let mut case = base(
        id,
        "synthesize two generated source notes with traceable citations",
        format!(
            "Read both source files in evidence-{index}/ and write findings.md with the two facts and citations [1] and [2]."
        ),
        vec![
            ScriptedTurn::tool_calls(vec![
                (
                    "read",
                    serde_json::json!({"path": format!("evidence-{index}/a.txt")}),
                ),
                (
                    "read",
                    serde_json::json!({"path": format!("evidence-{index}/b.txt")}),
                ),
            ]),
            ScriptedTurn::tool(
                "write",
                serde_json::json!({"path": "findings.md", "content": report}),
            ),
            ScriptedTurn::Text("The cited findings are ready.".into()),
        ],
        format!(
            "grep -q '{first} communities' findings.md && grep -q '{second} percent' findings.md && grep -q '\\[1\\]' findings.md && grep -q '\\[2\\]' findings.md"
        ),
    );
    case.files = vec![
        (format!("evidence-{index}/a.txt"), source_a),
        (format!("evidence-{index}/b.txt"), source_b),
    ];
    case
}

fn writing_case(id: String, entropy: u64, index: u64) -> EvalCase {
    const SUBJECTS: [&str; 6] = [
        "community gardens",
        "public libraries",
        "river paths",
        "shared kitchens",
        "local museums",
        "neighborhood markets",
    ];
    let subject = SUBJECTS[(entropy as usize) % SUBJECTS.len()];
    let title = format!("A note about {subject}");
    let content = format!(
        "# {title}\n\n## Purpose\nThis short brief describes why {subject} matter to the people who use them.\n\n## Benefits\nThey make useful services easier to reach, create welcoming places to meet, and help neighbors share knowledge and resources.\n\n## Next step\nInvite residents to name one practical improvement and choose a date to review it together.\n"
    );
    base(
        id,
        "write a structured, constrained brief about a generated everyday topic",
        format!(
            "Write brief-{index}.md about {subject}. Include a title, Purpose, Benefits, and Next step sections, each with useful prose."
        ),
        vec![
            ScriptedTurn::tool(
                "write",
                serde_json::json!({"path": format!("brief-{index}.md"), "content": content}),
            ),
            ScriptedTurn::Text("The brief is ready.".into()),
        ],
        format!(
            "grep -q '^# {title}' brief-{index}.md && grep -q '^## Purpose' brief-{index}.md && grep -q '^## Benefits' brief-{index}.md && grep -q '^## Next step' brief-{index}.md"
        ),
    )
}

fn conversion_case(id: String, entropy: u64, index: u64) -> EvalCase {
    const OWNERS: [&str; 8] = ["Ari", "Bea", "Chen", "Devi", "Eli", "Fatima", "Gus", "Hana"];
    const TASKS: [&str; 8] = [
        "send the summary",
        "book a room",
        "check the budget",
        "share the notes",
        "confirm the venue",
        "review the draft",
        "prepare the handout",
        "update the list",
    ];
    let a = (entropy as usize) % OWNERS.len();
    let b = (mix(entropy) as usize) % OWNERS.len();
    let task_a = TASKS[(mix(entropy ^ 0x1111) as usize) % TASKS.len()];
    let task_b = TASKS[(mix(entropy ^ 0x2222) as usize) % TASKS.len()];
    let date = format!(
        "2026-{:02}-{:02}",
        1 + (entropy % 12),
        1 + (mix(entropy) % 28)
    );
    let notes = format!(
        "Meeting {date}\n- {task_a} ({})\n- {task_b} ({})\n",
        OWNERS[a], OWNERS[b]
    );
    let json = serde_json::json!({
        "date": date,
        "items": [
            {"topic": task_a, "owner": OWNERS[a]},
            {"topic": task_b, "owner": OWNERS[b]},
        ]
    })
    .to_string();
    let mut case = base(
        id,
        "convert generated meeting notes to structured JSON while retaining owners",
        format!(
            "Read notes-{index}.txt and convert them into agenda-{index}.json with a date and an items array containing topic and owner."
        ),
        vec![
            ScriptedTurn::tool(
                "read",
                serde_json::json!({"path": format!("notes-{index}.txt")}),
            ),
            ScriptedTurn::tool(
                "write",
                serde_json::json!({"path": format!("agenda-{index}.json"), "content": json}),
            ),
            ScriptedTurn::Text("The agenda is converted.".into()),
        ],
        format!(
            "python3 -c \"import json; d=json.load(open('agenda-{index}.json')); assert d['date']=='{date}'; assert len(d['items'])==2; assert d['items'][0]['owner']=='{}'; assert d['items'][1]['owner']=='{}'\"",
            OWNERS[a], OWNERS[b]
        ),
    );
    case.files.push((format!("notes-{index}.txt"), notes));
    case
}

fn schedule_case(id: String, entropy: u64, index: u64) -> EvalCase {
    const PEOPLE: [&str; 8] = ["Ari", "Bea", "Chen", "Devi", "Eli", "Fatima", "Gus", "Hana"];
    let a = (entropy as usize) % PEOPLE.len();
    let b = (mix(entropy) as usize) % PEOPLE.len();
    let start_a = 9 + (mix(entropy ^ 0x3333) % 5) as u8;
    let end_a = start_a + 3;
    let start_b = start_a + 1;
    let end_b = start_b + 2;
    let shared = format!("{:02}:00-{:02}:00", start_b, end_a.min(end_b));
    let availability = format!(
        "{}: {:02}:00-{:02}:00\n{}: {:02}:00-{:02}:00\n",
        PEOPLE[a], start_a, end_a, PEOPLE[b], start_b, end_b
    );
    let mut case = base(
        id,
        "find a real overlap in generated availability and save a shared meeting time",
        format!(
            "Read availability-{index}.txt and write meeting-{index}.md with a time both people can attend."
        ),
        vec![
            ScriptedTurn::tool(
                "read",
                serde_json::json!({"path": format!("availability-{index}.txt")}),
            ),
            ScriptedTurn::tool(
                "write",
                serde_json::json!({"path": format!("meeting-{index}.md"), "content": format!("# Shared meeting\n\n{shared}\n")}),
            ),
            ScriptedTurn::Text("The shared time is saved.".into()),
        ],
        format!("grep -q '{shared}' meeting-{index}.md"),
    );
    case.files
        .push((format!("availability-{index}.txt"), availability));
    case
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cases_are_stable_distinct_and_cover_general_work() {
        let a = (0..25)
            .map(|index| generated_scenario(17, index))
            .collect::<Vec<_>>();
        let again = generated_scenario(17, 0);
        let other_seed = generated_scenario(18, 0);
        assert_eq!(a[0].id, again.id);
        assert_eq!(a[0].files, again.files);
        assert_ne!(a[0].files, other_seed.files);
        assert_eq!(
            a.iter()
                .map(|case| &case.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            a.len()
        );
        for family in FAMILIES {
            assert!(
                a.iter().any(|case| case.id.contains(family)),
                "missing {family}"
            );
        }
    }
}
