use super::*;
use crate::{JobAccess, JobType, LeadProposal, ProposalStatus, ProposedJob};
use serde_json::json;

fn job(title: Option<&str>, prompt: &str, output: &str) -> JobRecord {
    serde_json::from_value(json!({
        "id": "job",
        "type": "review",
        "status": "done",
        "title": title,
        "prompt": prompt,
        "output": output
    }))
    .unwrap()
}

#[test]
fn all_words_can_match_across_fields() {
    let job = job(Some("Draft"), "Check formatting", "Ready");
    assert!(SearchQuery::new("draft ready").job(&job, 1).is_some());
    assert!(SearchQuery::new("draft missing").job(&job, 1).is_none());
    assert!(SearchQuery::new("  ").job(&job, 1).is_none());
}

#[test]
fn matching_ignores_case_and_marks_original_text() {
    let job = job(None, "", "Keep CAFÉ and Café in context");
    let hit = SearchQuery::new("café").job(&job, 1).unwrap();
    assert_eq!(
        hit.snippet,
        vec![
            SnippetPart {
                text: "Keep ".into(),
                marked: false
            },
            SnippetPart {
                text: "CAFÉ".into(),
                marked: true
            },
            SnippetPart {
                text: " and ".into(),
                marked: false
            },
            SnippetPart {
                text: "Café".into(),
                marked: true
            },
            SnippetPart {
                text: " in context".into(),
                marked: false
            },
        ]
    );
}

#[test]
fn title_only_hit_keeps_a_matching_message_when_available() {
    let mut message: LeadMessage = serde_json::from_value(json!({
        "id": "message",
        "role": "lead",
        "text": "",
        "status": "done"
    }))
    .unwrap();
    message.proposal = Some(LeadProposal {
        summary: "Draft the plan".into(),
        jobs: vec![ProposedJob {
            title: "Proofread".into(),
            job_type: JobType::Review,
            access: JobAccess::Read,
            worker: None,
            prompt: "Check spacing".into(),
        }],
        status: ProposalStatus::Waiting,
        job_ids: None,
    });
    let hit = SearchQuery::new("draft plan")
        .chat("chat", "Draft plan", 1, &[message])
        .unwrap();
    assert_eq!(hit.message_id.as_deref(), Some("message"));
}

#[test]
fn untitled_job_prefers_a_matching_output_over_prompt() {
    let job = job(None, "Review this draft", "Completed the review");
    let hit = SearchQuery::new("review draft").job(&job, 1).unwrap();
    assert_eq!(hit.snippet[0].text, "Completed the ");
}

#[test]
fn newest_hits_come_first_across_kinds() {
    let query = SearchQuery::new("draft");
    let job = job(None, "Draft", "");
    let mut hits = vec![
        query.chat("older", "Draft", 10, &[]).unwrap(),
        query.job(&job, 30).unwrap(),
        query.chat("newer", "Draft", 20, &[]).unwrap(),
    ];
    sort_hits(&mut hits);
    assert_eq!(
        hits.iter().map(|hit| hit.at).collect::<Vec<_>>(),
        [30, 20, 10]
    );
}
