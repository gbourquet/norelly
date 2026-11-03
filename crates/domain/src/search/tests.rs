use proptest::prelude::*;

use super::*;

fn parse(input: &str) -> Vec<Clause> {
    parse_query(input)
        .unwrap_or_else(|e| panic!("requête {input:?} refusée : {e}"))
        .clauses()
        .to_vec()
}

fn word(field: Field, text: &str) -> Clause {
    Clause {
        negated: false,
        atom: Atom::Text {
            field,
            text: text.to_owned(),
            phrase: false,
        },
    }
}

fn phrase(field: Field, text: &str) -> Clause {
    Clause {
        negated: false,
        atom: Atom::Text {
            field,
            text: text.to_owned(),
            phrase: true,
        },
    }
}

fn filter(filter: Filter) -> Clause {
    Clause {
        negated: false,
        atom: Atom::Filter(filter),
    }
}

fn not(clause: Clause) -> Clause {
    Clause {
        negated: true,
        ..clause
    }
}

fn date(year: i32, month: u8, day: u8) -> Date {
    Date::new(year, month, day).unwrap_or_else(|| panic!("date {year}-{month}-{day}"))
}

#[test]
fn free_words_are_searched_everywhere() {
    assert_eq!(
        parse("facture  avril"),
        [word(Field::Any, "facture"), word(Field::Any, "avril")]
    );
}

#[test]
fn empty_or_blank_query_has_no_clause() {
    assert!(parse_query("").is_ok_and(|q| q.is_empty()));
    assert!(parse_query("  \t ").is_ok_and(|q| q.is_empty()));
}

#[test]
fn quoted_text_is_a_phrase() {
    assert_eq!(
        parse("\"compte rendu\" réunion"),
        [
            phrase(Field::Any, "compte rendu"),
            word(Field::Any, "réunion")
        ]
    );
}

#[test]
fn unterminated_quote_runs_to_the_end() {
    assert_eq!(parse("\"bon de comm"), [phrase(Field::Any, "bon de comm")]);
}

#[test]
fn empty_quotes_are_ignored() {
    assert_eq!(parse("\"\" \"  \" a"), [word(Field::Any, "a")]);
}

#[test]
fn field_operators_target_a_field() {
    assert_eq!(
        parse("from:alice to:bob@example.com subject:devis"),
        [
            word(Field::From, "alice"),
            word(Field::To, "bob@example.com"),
            word(Field::Subject, "devis"),
        ]
    );
}

#[test]
fn field_operators_accept_a_phrase() {
    assert_eq!(
        parse("subject:\"ordre du jour\" from:\"Alice Martin\""),
        [
            phrase(Field::Subject, "ordre du jour"),
            phrase(Field::From, "Alice Martin"),
        ]
    );
}

#[test]
fn operators_ignore_case() {
    assert_eq!(
        parse("FROM:alice Is:Unread HAS:Attachment"),
        [
            word(Field::From, "alice"),
            filter(Filter::Unread),
            filter(Filter::HasAttachment),
        ]
    );
}

#[test]
fn property_filters() {
    assert_eq!(
        parse("has:attachment is:unread is:read is:flagged"),
        [
            filter(Filter::HasAttachment),
            filter(Filter::Unread),
            filter(Filter::Read),
            filter(Filter::Flagged),
        ]
    );
}

#[test]
fn date_filters() {
    assert_eq!(
        parse("after:2026-01-01 before:2026-02-29"),
        [
            filter(Filter::After(date(2026, 1, 1))),
            word(Field::Any, "before:2026-02-29")
        ]
    );
    assert_eq!(
        parse("before:2024-02-29"),
        [filter(Filter::Before(date(2024, 2, 29)))]
    );
}

#[test]
fn unknown_or_invalid_operators_are_plain_text() {
    assert_eq!(
        parse("is:bleu has:chat before:hier foo:bar after:2026-13-01"),
        [
            word(Field::Any, "is:bleu"),
            word(Field::Any, "has:chat"),
            word(Field::Any, "before:hier"),
            word(Field::Any, "foo:bar"),
            word(Field::Any, "after:2026-13-01"),
        ]
    );
}

#[test]
fn operator_without_value_is_plain_text() {
    assert_eq!(
        parse("from: alice"),
        [word(Field::Any, "from:"), word(Field::Any, "alice")]
    );
    assert_eq!(parse("subject:\"\""), [word(Field::Any, "subject:")]);
}

#[test]
fn minus_negates_any_clause() {
    assert_eq!(
        parse("-spam -from:bob -\"lettre d'info\" -is:unread"),
        [
            not(word(Field::Any, "spam")),
            not(word(Field::From, "bob")),
            not(phrase(Field::Any, "lettre d'info")),
            not(filter(Filter::Unread)),
        ]
    );
}

#[test]
fn lone_minus_is_ignored_and_inner_minus_is_kept() {
    assert_eq!(
        parse("- jean-pierre --x"),
        [word(Field::Any, "jean-pierre"), not(word(Field::Any, "-x"))]
    );
}

#[test]
fn too_long_query_is_refused() {
    let long = "a".repeat(MAX_QUERY_CHARS + 1);
    assert_eq!(parse_query(&long), Err(QueryError::TooLong));
    let exact = "é".repeat(MAX_QUERY_CHARS);
    assert!(
        parse_query(&exact).is_ok(),
        "la borne compte des caractères"
    );
}

#[test]
fn too_many_clauses_are_refused() {
    let many = vec!["a"; MAX_CLAUSES + 1].join(" ");
    assert_eq!(parse_query(&many), Err(QueryError::TooManyClauses));
    let enough = vec!["a"; MAX_CLAUSES].join(" ");
    assert_eq!(parse(&enough).len(), MAX_CLAUSES);
}

#[test]
fn dates_are_validated() {
    assert!(Date::new(2026, 2, 28).is_some());
    assert!(Date::new(2026, 2, 29).is_none());
    assert!(Date::new(2000, 2, 29).is_some());
    assert!(Date::new(1900, 2, 29).is_none());
    assert!(Date::new(2026, 4, 31).is_none());
    assert!(Date::new(2026, 0, 1).is_none());
    assert!(Date::new(2026, 1, 0).is_none());
    assert!(Date::new(0, 1, 1).is_none());
    assert!(Date::new(10_000, 1, 1).is_none());
}

#[test]
fn day_start_honours_the_utc_offset() {
    assert_eq!(date(1970, 1, 1).start_timestamp(0), 0);
    assert_eq!(date(2026, 11, 3).start_timestamp(0), 1_793_664_000);
    // Paris en hiver (UTC+1) : le jour commence une heure plus tôt.
    assert_eq!(date(2026, 11, 3).start_timestamp(3600), 1_793_660_400);
    assert_eq!(date(1969, 12, 31).start_timestamp(0), -86_400);
}

proptest! {
    #[test]
    fn parsing_never_panics(input in "\\PC{0,600}") {
        let _ = parse_query(&input);
    }

    #[test]
    fn parsing_never_panics_on_syntax_soup(
        input in "([-\" :]|from|to|subject|is|has|before|after|unread|2026-01-0[1-9]|[a-zé]{1,3}){0,40}"
    ) {
        if let Ok(query) = parse_query(&input) {
            prop_assert!(query.clauses().len() <= MAX_CLAUSES);
            for clause in query.clauses() {
                if let Atom::Text { text, .. } = &clause.atom {
                    prop_assert!(!text.trim().is_empty());
                }
            }
        }
    }

    #[test]
    fn day_starts_are_one_day_apart(year in 1i32..9999, month in 1u8..=12, day in 1u8..=28) {
        let d = date(year, month, day);
        let next = Date::new(year, month, day + 1).unwrap_or(d);
        if next != d {
            prop_assert_eq!(next.start_timestamp(0) - d.start_timestamp(0), 86_400);
        }
    }
}

fn seg(text: &str, hit: bool) -> Segment {
    Segment {
        text: text.to_owned(),
        hit,
    }
}

fn terms(query: &str) -> Vec<String> {
    parse_query(query)
        .unwrap_or_else(|e| panic!("requête {query:?} refusée : {e}"))
        .highlight_terms()
}

#[test]
fn folding_ignores_case_and_diacritics() {
    assert_eq!(
        fold("Élève À l'ÉCOLE, ça çà Œuvre ñ Ü ß"),
        "eleve a l'ecole, ca ca œuvre n u ß"
    );
    // Comme `unicode61` : seules les lettres décomposables sont pliées.
    assert_eq!(fold("Ångström Łódź Øre"), "angstrom łodz øre");
}

#[test]
fn highlight_terms_are_the_folded_words_of_positive_text_clauses() {
    assert_eq!(
        terms("Réunion from:Alice -spam \"Compte-rendu\" is:unread subject:\"ordre du\""),
        ["reunion", "alice", "compte", "rendu", "ordre", "du"]
    );
}

#[test]
fn highlight_marks_words_starting_with_a_term() {
    assert_eq!(
        highlight("Facture d'avril : la facturation", &terms("fact")),
        [
            seg("Facture", true),
            seg(" d'avril : la ", false),
            seg("facturation", true),
        ]
    );
}

#[test]
fn highlight_ignores_diacritics_and_inner_matches() {
    assert_eq!(
        highlight("Réunion préparatoire", &terms("reu para")),
        [seg("Réunion", true), seg(" préparatoire", false)]
    );
}

#[test]
fn highlight_without_hit_is_one_plain_segment() {
    assert_eq!(highlight("Bonjour", &terms("zz")), [seg("Bonjour", false)]);
    assert!(highlight("", &terms("zz")).is_empty());
}

#[test]
fn highlight_merges_adjacent_hits_with_their_separator_kept_apart() {
    assert_eq!(
        highlight("compte-rendu final", &terms("\"compte rendu\"")),
        [
            seg("compte", true),
            seg("-", false),
            seg("rendu", true),
            seg(" final", false),
        ]
    );
}

proptest! {
    #[test]
    fn highlight_preserves_the_text(text in "\\PC{0,200}", query in "\\PC{0,40}") {
        let terms = parse_query(&query).map(|q| q.highlight_terms()).unwrap_or_default();
        let segments = highlight(&text, &terms);
        let joined: String = segments.iter().map(|s| s.text.as_str()).collect();
        prop_assert_eq!(joined, text);
        prop_assert!(segments.iter().all(|s| !s.text.is_empty()));
        prop_assert!(segments.windows(2).all(|w| w[0].hit != w[1].hit));
    }
}

#[test]
fn tokens_are_folded_alphanumeric_runs() {
    assert_eq!(
        tokens("Compte-rendu : RÉUNION n°2, alice@example.com"),
        ["compte", "rendu", "reunion", "n", "2", "alice", "example", "com"]
    );
    assert!(tokens(" -- ! ").is_empty());
}
