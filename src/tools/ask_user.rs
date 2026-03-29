//! AskUserQuestion tool for structured user interaction.
//!
//! This module provides types and functions for presenting structured questions
//! to the user and validating their responses. Questions can be free-text or
//! provide a list of choices for the user to select from.
//!
//! The tool is intercepted by [`AppState`](crate::app::state::AppState) rather
//! than executed through the normal tool pipeline, because it requires modal UI
//! interaction via the [`QuestionHandler`](crate::app::handlers::question::QuestionHandler).
//!
//! # Examples
//!
//! ```
//! use patina::tools::ask_user::{QuestionRequest, format_question, validate_answer};
//!
//! let request = QuestionRequest {
//!     question: "Which framework?".into(),
//!     choices: Some(vec!["React".into(), "Vue".into(), "Svelte".into()]),
//!     default: Some("React".into()),
//! };
//!
//! let display = format_question(&request);
//! assert!(display.contains("Which framework?"));
//!
//! let answer = validate_answer(&request, "Vue").unwrap();
//! assert_eq!(answer.answer, "Vue");
//! assert_eq!(answer.choice_index, Some(1));
//! ```

use anyhow::{bail, Result};

/// A structured question to present to the user.
///
/// Parsed from tool parameters when the LLM invokes the `ask_user` tool.
/// Supports both free-text and multiple-choice modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserQuestion {
    /// The question text to display to the user.
    pub question: String,
    /// Optional list of choices the user can select from.
    pub choices: Option<Vec<String>>,
    /// Optional default answer if the user submits without input.
    pub default: Option<String>,
}

/// A validated user answer to a question.
///
/// Contains the raw answer text and, when the question had choices,
/// the index of the matched choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAnswer {
    /// The user's answer text.
    pub answer: String,
    /// The index of the selected choice, if the question had choices
    /// and the answer matched one of them.
    pub choice_index: Option<usize>,
}

/// Parsed request from tool parameters for the `ask_user` tool.
///
/// This is the input representation extracted from the JSON tool parameters.
/// It maps directly to the fields the LLM provides when invoking the tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRequest {
    /// The question text to present.
    pub question: String,
    /// Optional list of choices for the user.
    pub choices: Option<Vec<String>>,
    /// Optional default answer.
    pub default: Option<String>,
}

impl QuestionRequest {
    /// Parses a [`QuestionRequest`] from JSON tool input parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if the `question` field is missing or empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::tools::ask_user::QuestionRequest;
    /// use serde_json::json;
    ///
    /// let input = json!({
    ///     "question": "Which database?",
    ///     "choices": ["PostgreSQL", "MySQL"],
    ///     "default": "PostgreSQL"
    /// });
    /// let request = QuestionRequest::from_json(&input).unwrap();
    /// assert_eq!(request.question, "Which database?");
    /// ```
    pub fn from_json(input: &serde_json::Value) -> Result<Self> {
        let question = input
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if question.is_empty() {
            bail!("Question text must not be empty");
        }

        let choices = input
            .get("choices")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .and_then(|v| if v.is_empty() { None } else { Some(v) });

        let default = input
            .get("default")
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok(Self {
            question,
            choices,
            default,
        })
    }

    /// Converts this request into a [`UserQuestion`].
    #[must_use]
    pub fn to_user_question(&self) -> UserQuestion {
        UserQuestion {
            question: self.question.clone(),
            choices: self.choices.clone(),
            default: self.default.clone(),
        }
    }
}

/// Formats a question request for display in the terminal.
///
/// Produces a human-readable string with the question text, numbered choices
/// (if any), and the default value (if specified).
///
/// # Examples
///
/// ```
/// use patina::tools::ask_user::{QuestionRequest, format_question};
///
/// let request = QuestionRequest {
///     question: "Pick a color".into(),
///     choices: Some(vec!["Red".into(), "Blue".into()]),
///     default: Some("Red".into()),
/// };
/// let formatted = format_question(&request);
/// assert!(formatted.contains("Pick a color"));
/// assert!(formatted.contains("1) Red"));
/// assert!(formatted.contains("2) Blue"));
/// assert!(formatted.contains("[default: Red]"));
/// ```
#[must_use]
pub fn format_question(request: &QuestionRequest) -> String {
    let mut output = String::new();
    output.push_str(&request.question);

    if let Some(ref choices) = request.choices {
        output.push('\n');
        for (i, choice) in choices.iter().enumerate() {
            output.push_str(&format!("  {}) {choice}\n", i + 1));
        }
    }

    if let Some(ref default) = request.default {
        output.push_str(&format!("[default: {default}]"));
    }

    output
}

/// Validates a user answer against a question request.
///
/// If the question has choices, the answer must match one of them (case-insensitive).
/// If the answer is empty and a default is set, the default is used.
/// For free-text questions (no choices), any non-empty answer is valid.
///
/// # Errors
///
/// Returns an error if:
/// - The answer is empty and no default is provided
/// - The answer does not match any choice when choices are specified
///
/// # Examples
///
/// ```
/// use patina::tools::ask_user::{QuestionRequest, validate_answer};
///
/// let request = QuestionRequest {
///     question: "Pick one".into(),
///     choices: Some(vec!["A".into(), "B".into()]),
///     default: None,
/// };
/// let answer = validate_answer(&request, "A").unwrap();
/// assert_eq!(answer.choice_index, Some(0));
/// ```
pub fn validate_answer(request: &QuestionRequest, answer: &str) -> Result<UserAnswer> {
    let answer_text = answer.trim();

    // Handle empty answer with default fallback
    if answer_text.is_empty() {
        if let Some(ref default) = request.default {
            // If the default matches a choice, resolve the index
            let choice_index = request
                .choices
                .as_ref()
                .and_then(|choices| choices.iter().position(|c| c.eq_ignore_ascii_case(default)));
            return Ok(UserAnswer {
                answer: default.clone(),
                choice_index,
            });
        }
        bail!("Answer must not be empty (no default provided)");
    }

    // If choices are present, match against them
    if let Some(ref choices) = request.choices {
        // Try exact match first, then case-insensitive
        if let Some(idx) = choices.iter().position(|c| c == answer_text) {
            return Ok(UserAnswer {
                answer: choices[idx].clone(),
                choice_index: Some(idx),
            });
        }
        if let Some(idx) = choices
            .iter()
            .position(|c| c.eq_ignore_ascii_case(answer_text))
        {
            return Ok(UserAnswer {
                answer: choices[idx].clone(),
                choice_index: Some(idx),
            });
        }

        // Try numeric index (1-based)
        if let Ok(num) = answer_text.parse::<usize>() {
            if num >= 1 && num <= choices.len() {
                return Ok(UserAnswer {
                    answer: choices[num - 1].clone(),
                    choice_index: Some(num - 1),
                });
            }
        }

        bail!(
            "Invalid choice: '{}'. Expected one of: {}",
            answer_text,
            choices.join(", ")
        );
    }

    // Free-text mode: any non-empty answer is valid
    Ok(UserAnswer {
        answer: answer_text.to_string(),
        choice_index: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // QuestionRequest::from_json
    // =========================================================================

    #[test]
    fn from_json_parses_full_request() {
        let input = json!({
            "question": "Which database?",
            "choices": ["PostgreSQL", "MySQL", "SQLite"],
            "default": "PostgreSQL"
        });
        let request = QuestionRequest::from_json(&input).unwrap();
        assert_eq!(request.question, "Which database?");
        assert_eq!(
            request.choices,
            Some(vec![
                "PostgreSQL".to_string(),
                "MySQL".to_string(),
                "SQLite".to_string()
            ])
        );
        assert_eq!(request.default, Some("PostgreSQL".to_string()));
    }

    #[test]
    fn from_json_parses_minimal_request() {
        let input = json!({ "question": "What should I do?" });
        let request = QuestionRequest::from_json(&input).unwrap();
        assert_eq!(request.question, "What should I do?");
        assert!(request.choices.is_none());
        assert!(request.default.is_none());
    }

    #[test]
    fn from_json_rejects_empty_question() {
        let input = json!({ "question": "" });
        let err = QuestionRequest::from_json(&input).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn from_json_rejects_missing_question() {
        let input = json!({ "choices": ["a", "b"] });
        let err = QuestionRequest::from_json(&input).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn from_json_treats_empty_choices_as_none() {
        let input = json!({ "question": "Something?", "choices": [] });
        let request = QuestionRequest::from_json(&input).unwrap();
        assert!(request.choices.is_none());
    }

    // =========================================================================
    // to_user_question
    // =========================================================================

    #[test]
    fn to_user_question_copies_all_fields() {
        let request = QuestionRequest {
            question: "Which?".into(),
            choices: Some(vec!["A".into(), "B".into()]),
            default: Some("A".into()),
        };
        let uq = request.to_user_question();
        assert_eq!(uq.question, "Which?");
        assert_eq!(uq.choices, Some(vec!["A".to_string(), "B".to_string()]));
        assert_eq!(uq.default, Some("A".to_string()));
    }

    // =========================================================================
    // format_question — with choices
    // =========================================================================

    #[test]
    fn format_question_with_choices_and_default() {
        let request = QuestionRequest {
            question: "Pick a color".into(),
            choices: Some(vec!["Red".into(), "Blue".into(), "Green".into()]),
            default: Some("Red".into()),
        };
        let formatted = format_question(&request);
        assert!(formatted.contains("Pick a color"));
        assert!(formatted.contains("1) Red"));
        assert!(formatted.contains("2) Blue"));
        assert!(formatted.contains("3) Green"));
        assert!(formatted.contains("[default: Red]"));
    }

    #[test]
    fn format_question_with_choices_no_default() {
        let request = QuestionRequest {
            question: "Pick one".into(),
            choices: Some(vec!["X".into(), "Y".into()]),
            default: None,
        };
        let formatted = format_question(&request);
        assert!(formatted.contains("Pick one"));
        assert!(formatted.contains("1) X"));
        assert!(formatted.contains("2) Y"));
        assert!(!formatted.contains("[default"));
    }

    // =========================================================================
    // format_question — free-text (no choices)
    // =========================================================================

    #[test]
    fn format_question_free_text_with_default() {
        let request = QuestionRequest {
            question: "What name?".into(),
            choices: None,
            default: Some("World".into()),
        };
        let formatted = format_question(&request);
        assert!(formatted.contains("What name?"));
        assert!(formatted.contains("[default: World]"));
        // Should not contain numbered choices
        assert!(!formatted.contains("1)"));
    }

    #[test]
    fn format_question_free_text_no_default() {
        let request = QuestionRequest {
            question: "Enter value".into(),
            choices: None,
            default: None,
        };
        let formatted = format_question(&request);
        assert_eq!(formatted, "Enter value");
    }

    // =========================================================================
    // validate_answer — valid choice
    // =========================================================================

    #[test]
    fn validate_answer_exact_choice_match() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["React".into(), "Vue".into(), "Svelte".into()]),
            default: None,
        };
        let answer = validate_answer(&request, "Vue").unwrap();
        assert_eq!(answer.answer, "Vue");
        assert_eq!(answer.choice_index, Some(1));
    }

    #[test]
    fn validate_answer_case_insensitive_choice_match() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["React".into(), "Vue".into()]),
            default: None,
        };
        let answer = validate_answer(&request, "react").unwrap();
        assert_eq!(answer.answer, "React");
        assert_eq!(answer.choice_index, Some(0));
    }

    #[test]
    fn validate_answer_numeric_choice_selection() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into(), "B".into(), "C".into()]),
            default: None,
        };
        let answer = validate_answer(&request, "2").unwrap();
        assert_eq!(answer.answer, "B");
        assert_eq!(answer.choice_index, Some(1));
    }

    // =========================================================================
    // validate_answer — invalid choice
    // =========================================================================

    #[test]
    fn validate_answer_rejects_invalid_choice() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into(), "B".into()]),
            default: None,
        };
        let err = validate_answer(&request, "C").unwrap_err();
        assert!(err.to_string().contains("Invalid choice"));
        assert!(err.to_string().contains("A, B"));
    }

    #[test]
    fn validate_answer_rejects_out_of_range_numeric() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into(), "B".into()]),
            default: None,
        };
        let err = validate_answer(&request, "3").unwrap_err();
        assert!(err.to_string().contains("Invalid choice"));
    }

    #[test]
    fn validate_answer_rejects_zero_numeric() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into()]),
            default: None,
        };
        let err = validate_answer(&request, "0").unwrap_err();
        assert!(err.to_string().contains("Invalid choice"));
    }

    // =========================================================================
    // validate_answer — default answer
    // =========================================================================

    #[test]
    fn validate_answer_uses_default_when_empty() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into(), "B".into()]),
            default: Some("B".into()),
        };
        let answer = validate_answer(&request, "").unwrap();
        assert_eq!(answer.answer, "B");
        assert_eq!(answer.choice_index, Some(1));
    }

    #[test]
    fn validate_answer_uses_default_when_whitespace_only() {
        let request = QuestionRequest {
            question: "Name?".into(),
            choices: None,
            default: Some("default-name".into()),
        };
        let answer = validate_answer(&request, "   ").unwrap();
        assert_eq!(answer.answer, "default-name");
        assert!(answer.choice_index.is_none());
    }

    #[test]
    fn validate_answer_default_without_matching_choice() {
        let request = QuestionRequest {
            question: "Name?".into(),
            choices: Some(vec!["X".into(), "Y".into()]),
            default: Some("Z".into()),
        };
        let answer = validate_answer(&request, "").unwrap();
        assert_eq!(answer.answer, "Z");
        assert!(answer.choice_index.is_none());
    }

    // =========================================================================
    // validate_answer — free-text (no choices)
    // =========================================================================

    #[test]
    fn validate_answer_free_text_accepts_any_input() {
        let request = QuestionRequest {
            question: "What name?".into(),
            choices: None,
            default: None,
        };
        let answer = validate_answer(&request, "MyProject").unwrap();
        assert_eq!(answer.answer, "MyProject");
        assert!(answer.choice_index.is_none());
    }

    #[test]
    fn validate_answer_free_text_trims_whitespace() {
        let request = QuestionRequest {
            question: "Name?".into(),
            choices: None,
            default: None,
        };
        let answer = validate_answer(&request, "  hello  ").unwrap();
        assert_eq!(answer.answer, "hello");
    }

    // =========================================================================
    // validate_answer — empty rejected without default
    // =========================================================================

    #[test]
    fn validate_answer_rejects_empty_without_default() {
        let request = QuestionRequest {
            question: "Name?".into(),
            choices: None,
            default: None,
        };
        let err = validate_answer(&request, "").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn validate_answer_rejects_empty_with_choices_no_default() {
        let request = QuestionRequest {
            question: "Pick".into(),
            choices: Some(vec!["A".into()]),
            default: None,
        };
        let err = validate_answer(&request, "").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    // =========================================================================
    // UserQuestion and UserAnswer are Send + Sync
    // =========================================================================

    #[test]
    fn user_question_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<UserQuestion>();
    }

    #[test]
    fn user_answer_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<UserAnswer>();
    }

    #[test]
    fn question_request_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<QuestionRequest>();
    }
}
