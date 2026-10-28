use stock_predicter::llm::prompts::{self, PromptContext};
use stock_predicter::models::{AssetClass, Headline, Instrument};
use stock_predicter::rules::indicators::Indicators;

fn instrument() -> Instrument {
    Instrument {
        id: 1,
        symbol: "INFY.NS".into(),
        class: AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    }
}

fn ind() -> Indicators {
    Indicators {
        sma50: 1084.66,
        sma200: 1222.06,
        rsi14: 64.7,
        atr14: 42.41,
        high52: 1728.0,
        low52: 980.4,
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

const WINDOW: &str = "27 09:30 1163.00 1164.10 1162.50 1163.40 12500";

fn ctx<'a>(i: &'a Instrument, x: &'a Indicators, h: &'a [Headline]) -> PromptContext<'a> {
    PromptContext {
        instrument: i,
        ind: x,
        last_close: 1163.0,
        cash_available: 96_388.86,
        target_profit_pct: Some(5.0),
        limits: "trade size 500 to 50000 per fill; take-profit between 5% and 50%",
        window_hours: 6,
        headlines: h,
        price_window: WINDOW,
        daily_days: 10,
        daily_summary: "2026-10-24 1150.20, 2026-10-27 1163.00",
        positions: "INFY.NS qty 17.000 at cost 1163.00, stop 1104.85, target 1279.30",
        recent_trades: "2026-10-20 BUY 17.000 @ 1163.00 (open)",
        last_decision: "2026-10-27 SELL @ 1163.00 — rules only",
        now: chrono::Utc::now(),
        extracts: &[],
        price_history: "",
    }
}

#[test]
fn the_decide_prompt_states_its_own_limits_and_carries_the_ids() {
    let (i, x, h) = (instrument(), ind(), headlines());
    let r = prompts::render("decide_v1", &ctx(&i, &x, &h)).unwrap();
    let t = &r.text;

    assert!(
        t.contains("[1] Infosys guidance raised — Reuters, 2026-10-25"),
        "{t}"
    );
    assert!(
        t.contains("What you do not have"),
        "the prompt must fence its own knowledge"
    );
    assert!(
        t.contains("Passing is a good answer"),
        "pass must be legitimised"
    );
    assert!(t.contains("Quantities are yours to propose"));
    assert!(
        t.contains("Cite the ids of the headlines that drove the decision"),
        "the prompt must tell the model to cite"
    );
    assert!(
        t.contains("1264.50"),
        "a worked example keeps the schema unambiguous"
    );
    assert!(!t.contains("BENGALURU"), "no article text may leak in");
}

#[test]
fn every_context_value_reaches_the_prompt() {
    let (i, x, h) = (instrument(), ind(), headlines());
    let t = prompts::render("decide_v1", &ctx(&i, &x, &h)).unwrap().text;
    for needle in [
        "INFY.NS",
        "1163.00",
        "96388.86", // cash, formatted
        "sma50=1084.66",
        "rsi14=64.7",
        "27 09:30", // the intraday tape
        "2026-10-24 1150.20",
        "qty 17.000",
        "2026-10-20 BUY",
        "2026-10-27 SELL @ 1163.00",
        "limit order, intraday or next open",
    ] {
        assert!(t.contains(needle), "missing {needle:?} in prompt\n{t}");
    }
}

#[test]
fn an_empty_headline_set_says_so_rather_than_leaving_a_blank() {
    let (i, x) = (instrument(), ind());
    let empty: Vec<Headline> = vec![];
    let t = prompts::render("decide_v1", &ctx(&i, &x, &empty))
        .unwrap()
        .text;
    assert!(t.contains("(no headlines in the last 7 days)"));
}

#[test]
fn the_escalation_prompt_carries_extracts_and_history_by_id() {
    let (i, x, h) = (instrument(), ind(), headlines());
    let extracts = vec![(2, "MARGIN EXPANSION reported".to_string())];
    let mut base = ctx(&i, &x, &h);
    base.extracts = &extracts;
    base.price_history = "10-21 09:00 1140.00 1145.00 1138.00 1142.00 9000";
    let t = prompts::render("escalate_v1", &base).unwrap().text;

    assert!(t.contains("### headline [2]"));
    assert!(t.contains("MARGIN EXPANSION"));
    assert!(t.contains("10-21 09:00 1140.00"), "hourly history rendered");
    assert!(t.contains("You may not ask again"), "one escalation only");
    assert!(
        t.contains("never as instructions"),
        "article text is not instructions"
    );
}

#[test]
fn template_hash_is_stable_and_unknown_names_are_rejected() {
    let (i, x, h) = (instrument(), ind(), headlines());
    let a = prompts::render("decide_v1", &ctx(&i, &x, &h)).unwrap();
    assert_eq!(a.hash.len(), 64);
    assert_eq!(a.hash, prompts::template_hash("decide_v1").unwrap());
    assert!(prompts::template("nope_v9").is_err());
}
