//! Id prefix and exact-name lookup from `src/cli/ids.ts`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdItem {
    pub id: String,
    pub name: Option<String>,
}

impl IdItem {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
        }
    }

    pub fn named(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdHit {
    Found { id: String },
    Rejected { message: String },
}

/// Unique id prefix of at least 4 characters, or an exact name.
pub fn resolve_id(token: &str, items: &[IdItem], noun: &str) -> IdHit {
    let needle = token.to_lowercase();
    let exact: Vec<&IdItem> = items
        .iter()
        .filter(|item| item.id.to_lowercase() == needle)
        .collect();
    if exact.len() == 1 {
        return IdHit::Found {
            id: exact[0].id.clone(),
        };
    }

    let named: Vec<&IdItem> = items
        .iter()
        .filter(|item| match item.name.as_deref() {
            Some(name) if !name.is_empty() => name == token,
            _ => false,
        })
        .collect();
    let prefixed: Vec<&IdItem> = if token.chars().count() >= 4 {
        items
            .iter()
            .filter(|item| item.id.to_lowercase().starts_with(&needle))
            .collect()
    } else {
        Vec::new()
    };

    let mut matches: Vec<&IdItem> = Vec::new();
    for item in exact.into_iter().chain(named).chain(prefixed) {
        if !matches.iter().any(|found| found.id == item.id) {
            matches.push(item);
        }
    }

    if matches.len() == 1 {
        return IdHit::Found {
            id: matches[0].id.clone(),
        };
    }
    if matches.len() > 1 {
        let listed: Vec<String> = matches
            .iter()
            .map(|item| match item.name.as_deref() {
                Some(name) if !name.is_empty() => format!("{} ({name})", item.id),
                _ => item.id.clone(),
            })
            .collect();
        return IdHit::Rejected {
            message: format!("{noun} id \"{token}\" matches {}.", english_list(&listed)),
        };
    }
    if token.chars().count() < 4 {
        return IdHit::Rejected {
            message: format!("{noun} id needs at least 4 characters."),
        };
    }
    IdHit::Rejected {
        message: format!("No {} matches \"{token}\".", noun.to_lowercase()),
    }
}

fn english_list(parts: &[String]) -> String {
    if parts.len() <= 1 {
        return parts.first().cloned().unwrap_or_default();
    }
    if parts.len() == 2 {
        return format!("{} and {}", parts[0], parts[1]);
    }
    let last = parts.len() - 1;
    format!("{}, and {}", parts[..last].join(", "), parts[last])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jobs() -> Vec<IdItem> {
        vec![
            IdItem::new("abcd1111-2222"),
            IdItem::new("abcd9999-0000"),
            IdItem::new("ffff0000-1111"),
        ]
    }

    #[test]
    fn a_unique_prefix_of_at_least_4_characters_resolves() {
        assert_eq!(
            resolve_id("ffff", &jobs(), "Job"),
            IdHit::Found {
                id: "ffff0000-1111".to_string()
            }
        );
        assert_eq!(
            resolve_id("FFFF0000-1111", &jobs(), "Job"),
            IdHit::Found {
                id: "ffff0000-1111".to_string()
            }
        );
    }

    #[test]
    fn a_short_prefix_is_refused() {
        assert_eq!(
            resolve_id("abc", &jobs(), "Job"),
            IdHit::Rejected {
                message: "Job id needs at least 4 characters.".to_string()
            }
        );
    }

    #[test]
    fn an_ambiguous_prefix_lists_the_matches() {
        assert_eq!(
            resolve_id("abcd", &jobs(), "Job"),
            IdHit::Rejected {
                message: "Job id \"abcd\" matches abcd1111-2222 and abcd9999-0000.".to_string(),
            }
        );
    }

    #[test]
    fn an_exact_name_resolves_and_a_clash_with_another_id_is_ambiguous() {
        let changes = vec![
            IdItem::named("aaaa1111", "login"),
            IdItem::named("bbbb2222", "other"),
        ];
        assert_eq!(
            resolve_id("login", &changes, "Change"),
            IdHit::Found {
                id: "aaaa1111".to_string()
            }
        );
        let clash = vec![IdItem::named("aaaa1111", "aaaa"), IdItem::new("aaaa2222")];
        assert_eq!(
            resolve_id("aaaa", &clash, "Change"),
            IdHit::Rejected {
                message: "Change id \"aaaa\" matches aaaa1111 (aaaa) and aaaa2222.".to_string(),
            }
        );
    }

    #[test]
    fn a_token_that_matches_nothing_says_so() {
        assert_eq!(
            resolve_id("zzzz", &jobs(), "Change"),
            IdHit::Rejected {
                message: "No change matches \"zzzz\".".to_string()
            }
        );
    }
}
