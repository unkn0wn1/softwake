//! Argument parsers for inbox / calendar / Drive read tools.

use crate::ToolError;

/// Parsed `email_list` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailListArgs {
    /// Optional max results.
    pub max_results: Option<u32>,
}

/// Parsed `email_search` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSearchArgs {
    /// Provider query (Gmail `q` or Graph search text).
    pub query: String,
    /// Optional max results.
    pub max_results: Option<u32>,
}

/// Parsed `email_get` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailGetArgs {
    /// Message id.
    pub id: String,
}

/// Parsed `calendar_list` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarListArgs {
    /// Upcoming window in days.
    pub days: Option<u32>,
    /// Optional max events.
    pub max_results: Option<u32>,
}

/// Parsed `calendar_get` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarGetArgs {
    /// Event id.
    pub id: String,
}

/// Parsed `drive_list` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveListArgs {
    /// Optional max files.
    pub max_results: Option<u32>,
}

/// Parsed `drive_search` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveSearchArgs {
    /// Name / query text.
    pub query: String,
    /// Optional max files.
    pub max_results: Option<u32>,
}

/// Parsed `drive_get` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveGetArgs {
    /// File id.
    pub id: String,
    /// When true, fetch cheap text body when MIME allows.
    pub read_text: bool,
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn needs(name: &str) -> ToolError {
    ToolError::InvalidArgs {
        name: name.to_owned(),
    }
}

/// Parse `email_list` positional args: optional max.
///
/// # Errors
///
/// Non-numeric max.
pub fn parse_email_list_args(args: &[String]) -> Result<EmailListArgs, ToolError> {
    match args {
        [] => Ok(EmailListArgs { max_results: None }),
        [max] => {
            let max_results = parse_u32(max).ok_or_else(|| needs(crate::EMAIL_LIST_TOOL))?;
            Ok(EmailListArgs {
                max_results: Some(max_results),
            })
        }
        _ => Err(needs(crate::EMAIL_LIST_TOOL)),
    }
}

/// Parse `email_search`: query, optional max.
///
/// # Errors
///
/// Missing query.
pub fn parse_email_search_args(args: &[String]) -> Result<EmailSearchArgs, ToolError> {
    match args {
        [query] if !query.trim().is_empty() => Ok(EmailSearchArgs {
            query: query.clone(),
            max_results: None,
        }),
        [query, max] if !query.trim().is_empty() => {
            let max_results = parse_u32(max).ok_or_else(|| needs(crate::EMAIL_SEARCH_TOOL))?;
            Ok(EmailSearchArgs {
                query: query.clone(),
                max_results: Some(max_results),
            })
        }
        _ => Err(needs(crate::EMAIL_SEARCH_TOOL)),
    }
}

/// Parse `email_get`: id.
///
/// # Errors
///
/// Missing id.
pub fn parse_email_get_args(args: &[String]) -> Result<EmailGetArgs, ToolError> {
    match args {
        [id] if !id.trim().is_empty() => Ok(EmailGetArgs { id: id.clone() }),
        _ => Err(needs(crate::EMAIL_GET_TOOL)),
    }
}

/// Parse `calendar_list`: optional days, optional max.
///
/// # Errors
///
/// Non-numeric values.
pub fn parse_calendar_list_args(args: &[String]) -> Result<CalendarListArgs, ToolError> {
    match args {
        [] => Ok(CalendarListArgs {
            days: None,
            max_results: None,
        }),
        [days] => {
            let days = parse_u32(days).ok_or_else(|| needs(crate::CALENDAR_LIST_TOOL))?;
            Ok(CalendarListArgs {
                days: Some(days),
                max_results: None,
            })
        }
        [days, max] => {
            let days = parse_u32(days).ok_or_else(|| needs(crate::CALENDAR_LIST_TOOL))?;
            let max_results = parse_u32(max).ok_or_else(|| needs(crate::CALENDAR_LIST_TOOL))?;
            Ok(CalendarListArgs {
                days: Some(days),
                max_results: Some(max_results),
            })
        }
        _ => Err(needs(crate::CALENDAR_LIST_TOOL)),
    }
}

/// Parse `calendar_get`: id.
///
/// # Errors
///
/// Missing id.
pub fn parse_calendar_get_args(args: &[String]) -> Result<CalendarGetArgs, ToolError> {
    match args {
        [id] if !id.trim().is_empty() => Ok(CalendarGetArgs { id: id.clone() }),
        _ => Err(needs(crate::CALENDAR_GET_TOOL)),
    }
}

/// Parse `drive_list`: optional max.
///
/// # Errors
///
/// Non-numeric max.
pub fn parse_drive_list_args(args: &[String]) -> Result<DriveListArgs, ToolError> {
    match args {
        [] => Ok(DriveListArgs { max_results: None }),
        [max] => {
            let max_results = parse_u32(max).ok_or_else(|| needs(crate::DRIVE_LIST_TOOL))?;
            Ok(DriveListArgs {
                max_results: Some(max_results),
            })
        }
        _ => Err(needs(crate::DRIVE_LIST_TOOL)),
    }
}

/// Parse `drive_search`: query, optional max.
///
/// # Errors
///
/// Missing query.
pub fn parse_drive_search_args(args: &[String]) -> Result<DriveSearchArgs, ToolError> {
    match args {
        [query] if !query.trim().is_empty() => Ok(DriveSearchArgs {
            query: query.clone(),
            max_results: None,
        }),
        [query, max] if !query.trim().is_empty() => {
            let max_results = parse_u32(max).ok_or_else(|| needs(crate::DRIVE_SEARCH_TOOL))?;
            Ok(DriveSearchArgs {
                query: query.clone(),
                max_results: Some(max_results),
            })
        }
        _ => Err(needs(crate::DRIVE_SEARCH_TOOL)),
    }
}

/// Parse `drive_get`: id, optional `read_text` flag (`text` / `true` / `1`).
///
/// # Errors
///
/// Missing id.
pub fn parse_drive_get_args(args: &[String]) -> Result<DriveGetArgs, ToolError> {
    match args {
        [id] if !id.trim().is_empty() => Ok(DriveGetArgs {
            id: id.clone(),
            read_text: false,
        }),
        [id, flag] if !id.trim().is_empty() => {
            let read_text = matches!(
                flag.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "text" | "yes"
            );
            Ok(DriveGetArgs {
                id: id.clone(),
                read_text,
            })
        }
        _ => Err(needs(crate::DRIVE_GET_TOOL)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_round_trip() {
        assert_eq!(
            parse_email_list_args(&[]).unwrap(),
            EmailListArgs { max_results: None }
        );
        assert_eq!(
            parse_email_search_args(&["from:ada".into(), "5".into()]).unwrap(),
            EmailSearchArgs {
                query: "from:ada".into(),
                max_results: Some(5)
            }
        );
        assert_eq!(parse_email_get_args(&["abc".into()]).unwrap().id, "abc");
        assert_eq!(
            parse_calendar_list_args(&["14".into(), "10".into()]).unwrap(),
            CalendarListArgs {
                days: Some(14),
                max_results: Some(10)
            }
        );
        assert!(
            parse_drive_get_args(&["f1".into(), "text".into()])
                .unwrap()
                .read_text
        );
        assert!(parse_email_search_args(&[]).is_err());
    }
}
