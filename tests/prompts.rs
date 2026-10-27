use stock_predicter::llm::prompts::{self, PromptContext};
use stock_predicter::models::{Action, AssetClass, Candidate, Headline, Instrument};
use stock_predicter::rules::indicators::Indicators;

fn instrument() -> Instrument {
    Instrument {
        id: 1,
        symbol: "INFY".into(),
        class: AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    }
}

fn ind() -> Indicators {
    Indicators {
        sma50: 100.0,
        sma200: 90.0,
        rsi14: 55.0,
        atr14: 2.0,
        high52: 130.0,
        low52: 80.0,
    }
}

fn candidate() -> Candidate {
    Candidate {
        instrument_id: 1,
        action: Action::Buy,
        reasons: vec!["trend up".into()],
        entry: 105.0,
        stop: Some(101.0),
        target: Some(111.0),
        qty: 95.0,
        timing_note: AssetClass::Stock.timing_note().into(),
        urgent: false,
    }
}

fn headlines() -> Vec<Headline> {
    vec![
        Headline {
            id: 1,
            title: "Infosys guidance raised".into(),
            publisher: "Reuters".into(),
            published_at: "2026-10-25T00:00:00Z".into(),
            url: "https://example.com/a".into(),
        },
        Headline {
            id: 2,
            title: "IT sector outlook".into(),
            publisher: "Mint".into(),
            published_at: "2026-10-24T00:00:00Z".into(),
            url: "https://example.com/b".into(),
        },
    ]
}

fn ctx<'a>(
    i: &'a Instrument,
    ind: &'a Indicators,
    c: &'a Candidate,
    h: &'a [Headline],
) -> PromptContext<'a> {
    PromptContext {
        instrument: i,
        ind,
        headlines: h,
        candidate: c,
        cash_available: 10_000.0,
        class_rules: AssetClass::Stock.timing_note(),
        search_summaries: &[],
        extracts: &[],
    }
}

#[test]
fn rendered_prompt_contains_headline_ids_and_no_article_text() {
    let (i, x, c, h) = (instrument(), ind(), candidate(), headlines());
    let r = prompts::render("decide_v1", &ctx(&i, &x, &c, &h)).unwrap();
    assert!(r.text.contains("[1] Infosys guidance raised — Reuters"), "{}", r.text);
    assert!(!r.text.contains("BENGALURU"));
    assert!(r.text.contains("INFY"));
    assert!(r.text.contains("rsi14=55.0"));
    assert!(r.text.contains("10000.00"), "cash not rendered");
}

#[test]
fn template_hash_is_stable_and_recorded() {
    let (i, x, c, h) = (instrument(), ind(), candidate(), headlines());
    let a = prompts::render("decide_v1", &ctx(&i, &x, &c, &h)).unwrap();
    assert_eq!(a.hash.len(), 64);
    assert_eq!(a.hash, prompts::template_hash("decide_v1").unwrap());
    assert_eq!(a.template, "decide_v1");
}

#[test]
fn unknown_template_is_an_error() {
    assert!(prompts::template("nope_v9").is_err());
}

#[test]
fn escalation_prompt_carries_extracts_by_id() {
    let (i, x, c, h) = (instrument(), ind(), candidate(), headlines());
    let extracts = vec![(2, "MARGIN EXPANSION reported".to_string())];
    let mut base = ctx(&i, &x, &c, &h);
    base.extracts = &extracts;
    let r = prompts::render("escalate_v1", &base).unwrap();
    assert!(r.text.contains("### headline [2]"));
    assert!(r.text.contains("MARGIN EXPANSION"));
    assert!(r.text.contains("You may not request more articles"));
}
