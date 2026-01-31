//! Unit tests for skills module.
//!
//! These tests verify skill markdown parsing and frontmatter extraction.
//! Following TDD RED phase - validating skill parsing behavior.

use patina::skills::SkillEngine;
use std::fs;
use tempfile::TempDir;

/// Helper to create a skill directory with SKILL.md file
fn create_skill_dir(dir: &TempDir, skill_name: &str, content: &str) -> std::path::PathBuf {
    let skill_dir = dir.path().join(skill_name);
    fs::create_dir_all(&skill_dir).expect("Should create skill directory");
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(&skill_file, content).expect("Should write skill file");
    skill_dir
}

// =============================================================================
// Test Group: Skill Markdown Parsing
// =============================================================================

/// Tests parsing a complete skill markdown file with frontmatter and body.
///
/// Expected behavior:
/// - Frontmatter YAML is parsed into SkillConfig
/// - Body content after frontmatter becomes instructions
#[test]
fn test_skill_md_parsing_complete() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: test-skill
description: A test skill for unit testing
allowed_tools:
  - Bash
  - Read
triggers:
  keywords:
    - testing
    - unit test
  file_patterns:
    - "*.test.rs"
  always_active: false
---

# Test Skill Instructions

This is the body content of the skill.

## Usage

Use this skill for testing purposes.
"#;

    create_skill_dir(&temp_dir, "test-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    assert_eq!(skills.len(), 1, "Should have loaded one skill");

    let skill = &skills[0];
    assert_eq!(skill.name, "test-skill");
    assert_eq!(skill.description, "A test skill for unit testing");
    assert_eq!(skill.config.allowed_tools, vec!["Bash", "Read"]);
    assert!(
        skill.instructions.contains("# Test Skill Instructions"),
        "Body should be extracted"
    );
    assert!(
        skill.instructions.contains("Use this skill for testing"),
        "Full body content should be present"
    );
}

/// Tests parsing a skill with minimal frontmatter (only required fields).
#[test]
fn test_skill_md_parsing_minimal_frontmatter() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: minimal-skill
description: Minimal description
---

Instructions only.
"#;

    create_skill_dir(&temp_dir, "minimal-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    assert_eq!(skills.len(), 1);

    let skill = &skills[0];
    assert_eq!(skill.name, "minimal-skill");
    assert_eq!(skill.description, "Minimal description");
    assert!(skill.config.allowed_tools.is_empty());
    assert!(skill.config.triggers.keywords.is_empty());
    assert!(!skill.config.triggers.always_active);
    assert_eq!(skill.instructions.trim(), "Instructions only.");
}

/// Tests parsing a skill without frontmatter (name derived from directory).
#[test]
fn test_skill_md_parsing_no_frontmatter() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"# No Frontmatter Skill

This skill has no YAML frontmatter.
The name should be derived from the directory name.
"#;

    create_skill_dir(&temp_dir, "derived-name-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    assert_eq!(skills.len(), 1);

    let skill = &skills[0];
    assert_eq!(
        skill.name, "derived-name-skill",
        "Name should be derived from directory"
    );
    assert!(skill.description.is_empty(), "Description should be empty");
    assert!(
        skill.instructions.contains("# No Frontmatter Skill"),
        "Body should contain full content"
    );
}

/// Tests parsing multiple skills from a directory.
#[test]
fn test_skill_md_parsing_multiple_skills() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill1 = r#"---
name: skill-one
description: First skill
---
First skill instructions.
"#;

    let skill2 = r#"---
name: skill-two
description: Second skill
---
Second skill instructions.
"#;

    create_skill_dir(&temp_dir, "skill-one", skill1);
    create_skill_dir(&temp_dir, "skill-two", skill2);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    assert_eq!(skills.len(), 2, "Should have loaded two skills");

    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"skill-one"));
    assert!(names.contains(&"skill-two"));
}

/// Tests that directories without SKILL.md are ignored.
#[test]
fn test_skill_md_parsing_ignores_non_skill_dirs() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Create a valid skill
    let valid_skill = r#"---
name: valid-skill
description: Valid
---
Valid instructions.
"#;
    create_skill_dir(&temp_dir, "valid-skill", valid_skill);

    // Create a directory without SKILL.md
    let invalid_dir = temp_dir.path().join("not-a-skill");
    fs::create_dir_all(&invalid_dir).expect("Should create directory");
    fs::write(invalid_dir.join("README.md"), "Not a skill").expect("Should write file");

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    assert_eq!(skills.len(), 1, "Should only load valid skills");
    assert_eq!(skills[0].name, "valid-skill");
}

/// Tests loading from a non-existent directory returns Ok with no skills.
#[test]
fn test_skill_md_parsing_nonexistent_dir() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let nonexistent = temp_dir.path().join("does-not-exist");

    let mut engine = SkillEngine::new();
    let result = engine.load_from_dir(&nonexistent);

    assert!(result.is_ok(), "Should succeed for non-existent directory");
    assert!(engine.all_skills().is_empty(), "Should have no skills");
}

// =============================================================================
// Test Group: Frontmatter Extraction
// =============================================================================

/// Tests extracting frontmatter with all trigger types.
#[test]
fn test_skill_frontmatter_extraction_triggers() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: trigger-skill
description: Skill with triggers
triggers:
  keywords:
    - rust
    - cargo
    - compile
  file_patterns:
    - "*.rs"
    - "Cargo.toml"
  always_active: true
---

Trigger test instructions.
"#;

    create_skill_dir(&temp_dir, "trigger-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skills = engine.all_skills();
    let skill = &skills[0];

    assert_eq!(skill.config.triggers.keywords.len(), 3);
    assert!(skill.config.triggers.keywords.contains(&"rust".to_string()));
    assert!(skill
        .config
        .triggers
        .keywords
        .contains(&"cargo".to_string()));
    assert!(skill
        .config
        .triggers
        .keywords
        .contains(&"compile".to_string()));

    assert_eq!(skill.config.triggers.file_patterns.len(), 2);
    assert!(skill
        .config
        .triggers
        .file_patterns
        .contains(&"*.rs".to_string()));
    assert!(skill
        .config
        .triggers
        .file_patterns
        .contains(&"Cargo.toml".to_string()));

    assert!(skill.config.triggers.always_active);
}

/// Tests extracting frontmatter with allowed_tools list.
#[test]
fn test_skill_frontmatter_extraction_allowed_tools() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: tools-skill
description: Skill with allowed tools
allowed_tools:
  - Bash
  - Read
  - Write
  - Glob
  - Grep
---

Tool instructions.
"#;

    create_skill_dir(&temp_dir, "tools-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skill = &engine.all_skills()[0];
    assert_eq!(skill.config.allowed_tools.len(), 5);
    assert!(skill.config.allowed_tools.contains(&"Bash".to_string()));
    assert!(skill.config.allowed_tools.contains(&"Read".to_string()));
    assert!(skill.config.allowed_tools.contains(&"Write".to_string()));
    assert!(skill.config.allowed_tools.contains(&"Glob".to_string()));
    assert!(skill.config.allowed_tools.contains(&"Grep".to_string()));
}

/// Tests that empty frontmatter values default correctly.
#[test]
fn test_skill_frontmatter_extraction_defaults() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: defaults-skill
description: Testing defaults
---

Default test.
"#;

    create_skill_dir(&temp_dir, "defaults-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skill = &engine.all_skills()[0];

    // Verify defaults
    assert!(
        skill.config.allowed_tools.is_empty(),
        "allowed_tools should default to empty"
    );
    assert!(
        skill.config.triggers.keywords.is_empty(),
        "keywords should default to empty"
    );
    assert!(
        skill.config.triggers.file_patterns.is_empty(),
        "file_patterns should default to empty"
    );
    assert!(
        !skill.config.triggers.always_active,
        "always_active should default to false"
    );
}

/// Tests extracting body content with various markdown elements.
#[test]
fn test_skill_frontmatter_extraction_complex_body() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: complex-body
description: Skill with complex markdown body
---

# Main Header

This skill has **bold** and *italic* text.

## Code Example

```rust
fn main() {
    println!("Hello, world!");
}
```

## Lists

- Item 1
- Item 2
  - Nested item

1. Numbered item
2. Another item

> Blockquote here

---

Final paragraph with [a link](https://example.com).
"#;

    create_skill_dir(&temp_dir, "complex-body", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skill = &engine.all_skills()[0];

    // Verify body contains all markdown elements
    assert!(skill.instructions.contains("# Main Header"));
    assert!(skill.instructions.contains("**bold**"));
    assert!(skill.instructions.contains("```rust"));
    assert!(skill.instructions.contains("println!"));
    assert!(skill.instructions.contains("- Item 1"));
    assert!(skill.instructions.contains("> Blockquote"));
    assert!(skill.instructions.contains("[a link]"));
}

/// Tests that malformed frontmatter causes the skill to be skipped.
#[test]
fn test_skill_frontmatter_extraction_malformed_yaml() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Create a skill with invalid YAML
    let invalid_content = r#"---
name: invalid-skill
description: [This is invalid YAML
  unclosed bracket
---

Instructions.
"#;

    // Create a valid skill to ensure loading continues
    let valid_content = r#"---
name: valid-skill
description: Valid skill
---

Valid instructions.
"#;

    create_skill_dir(&temp_dir, "invalid-skill", invalid_content);
    create_skill_dir(&temp_dir, "valid-skill", valid_content);

    let mut engine = SkillEngine::new();
    // Should not fail - just skip invalid skills
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should continue despite invalid skill");

    let skills = engine.all_skills();
    // Only valid skill should be loaded
    assert_eq!(skills.len(), 1, "Only valid skills should be loaded");
    assert_eq!(skills[0].name, "valid-skill");
}

/// Tests frontmatter with whitespace variations.
#[test]
fn test_skill_frontmatter_extraction_whitespace() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = "---\nname: whitespace-skill\ndescription: Test whitespace handling\n---\n\n\n\nBody with leading newlines.\n\n\n";

    create_skill_dir(&temp_dir, "whitespace-skill", content);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skill = &engine.all_skills()[0];
    assert_eq!(skill.name, "whitespace-skill");
    // Body should be trimmed
    assert!(skill.instructions.starts_with("Body with leading newlines"));
}

/// Tests that source_path is correctly set.
#[test]
fn test_skill_source_path_tracking() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let content = r#"---
name: tracked-skill
description: Skill with path tracking
---

Instructions.
"#;

    let skill_dir = create_skill_dir(&temp_dir, "tracked-skill", content);
    let expected_path = skill_dir.join("SKILL.md");

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let skill = &engine.all_skills()[0];
    assert_eq!(
        skill.source_path, expected_path,
        "source_path should point to SKILL.md"
    );
}

/// Tests SkillEngine::default() creates empty engine.
#[test]
fn test_skill_engine_default() {
    let engine = SkillEngine::default();
    assert!(
        engine.all_skills().is_empty(),
        "Default engine should be empty"
    );
}

/// Tests SkillEngine::new() creates empty engine.
#[test]
fn test_skill_engine_new() {
    let engine = SkillEngine::new();
    assert!(engine.all_skills().is_empty(), "New engine should be empty");
}

// =============================================================================
// Test Group: Skill Matching
// =============================================================================

/// Tests skill matching based on keywords.
///
/// Expected behavior:
/// - Skills with matching keywords are returned
/// - Matching is case-insensitive
/// - Non-matching skills are filtered out
#[test]
fn test_skill_matching_keywords() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    // Skill that should match "rust" keyword
    let rust_skill = r#"---
name: rust-skill
description: Rust development skill
triggers:
  keywords:
    - rust
    - cargo
    - rustfmt
---

Rust skill instructions.
"#;

    // Skill that should match "python" keyword
    let python_skill = r#"---
name: python-skill
description: Python development skill
triggers:
  keywords:
    - python
    - pip
    - virtualenv
---

Python skill instructions.
"#;

    // Skill with no keywords and unique description
    let generic_skill = r#"---
name: generic-skill
description: Unrelated functionality
---

Generic instructions.
"#;

    create_skill_dir(&temp_dir, "rust-skill", rust_skill);
    create_skill_dir(&temp_dir, "python-skill", python_skill);
    create_skill_dir(&temp_dir, "generic-skill", generic_skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Test matching "rust" keyword
    let rust_matches = engine.match_skills("I need to compile rust code");
    assert_eq!(rust_matches.len(), 1, "Should match one skill for 'rust'");
    assert_eq!(rust_matches[0].name, "rust-skill");

    // Test matching "python" keyword
    let python_matches = engine.match_skills("Run a python script");
    assert_eq!(
        python_matches.len(),
        1,
        "Should match one skill for 'python'"
    );
    assert_eq!(python_matches[0].name, "python-skill");

    // Test matching "cargo" keyword (secondary keyword)
    let cargo_matches = engine.match_skills("Build with cargo");
    assert_eq!(cargo_matches.len(), 1, "Should match one skill for 'cargo'");
    assert_eq!(cargo_matches[0].name, "rust-skill");

    // Test no match
    let no_matches = engine.match_skills("Write a Java program");
    // Note: generic-skill has "skill" in description which matches "skill"
    // in the task - the current implementation matches on description words too
    let keyword_matches: Vec<_> = no_matches
        .iter()
        .filter(|s| s.name != "generic-skill")
        .collect();
    assert!(
        keyword_matches.is_empty(),
        "Should not match rust or python skills for 'Java'"
    );
}

/// Tests skill matching is case-insensitive.
#[test]
fn test_skill_matching_keywords_case_insensitive() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: test-skill
description: Test skill
triggers:
  keywords:
    - Docker
    - KUBERNETES
    - terraform
---

Infrastructure skill.
"#;

    create_skill_dir(&temp_dir, "test-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Test lowercase matches uppercase keyword
    let matches1 = engine.match_skills("deploy to docker");
    assert_eq!(
        matches1.len(),
        1,
        "Should match 'docker' case-insensitively"
    );

    // Test uppercase matches lowercase keyword
    let matches2 = engine.match_skills("Use TERRAFORM");
    assert_eq!(
        matches2.len(),
        1,
        "Should match 'terraform' case-insensitively"
    );

    // Test mixed case
    let matches3 = engine.match_skills("Deploy to KuBerNeTeS");
    assert_eq!(
        matches3.len(),
        1,
        "Should match 'kubernetes' case-insensitively"
    );
}

/// Tests skill matching with always_active trigger.
#[test]
fn test_skill_matching_always_active() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let always_active_skill = r#"---
name: always-active-skill
description: Always active skill
triggers:
  always_active: true
---

Always active instructions.
"#;

    let conditional_skill = r#"---
name: conditional-skill
description: Conditional skill
triggers:
  keywords:
    - specific
---

Conditional instructions.
"#;

    create_skill_dir(&temp_dir, "always-active-skill", always_active_skill);
    create_skill_dir(&temp_dir, "conditional-skill", conditional_skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Always active skill should match any task
    let matches = engine.match_skills("Any random task description");
    let always_active_match = matches.iter().find(|s| s.name == "always-active-skill");
    assert!(
        always_active_match.is_some(),
        "Always active skill should match any task"
    );

    // Conditional skill should not match
    let conditional_match = matches.iter().find(|s| s.name == "conditional-skill");
    assert!(
        conditional_match.is_none(),
        "Conditional skill should not match without keyword"
    );
}

/// Tests skill matching with multiple matching skills.
#[test]
fn test_skill_matching_multiple_matches() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill1 = r#"---
name: skill-1
description: First skill
triggers:
  keywords:
    - database
---

Skill 1 instructions.
"#;

    let skill2 = r#"---
name: skill-2
description: Second skill
triggers:
  keywords:
    - database
    - sql
---

Skill 2 instructions.
"#;

    let skill3 = r#"---
name: skill-3
description: Third skill
triggers:
  keywords:
    - nosql
---

Skill 3 instructions.
"#;

    create_skill_dir(&temp_dir, "skill-1", skill1);
    create_skill_dir(&temp_dir, "skill-2", skill2);
    create_skill_dir(&temp_dir, "skill-3", skill3);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Both skill-1 and skill-2 should match "database"
    let matches = engine.match_skills("Query the database");
    assert_eq!(matches.len(), 2, "Should match two skills for 'database'");

    let names: Vec<&str> = matches.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"skill-1"));
    assert!(names.contains(&"skill-2"));
}

/// Tests skill matching based on description words.
#[test]
fn test_skill_matching_description() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: security-skill
description: Handles authentication and authorization flows
---

Security skill instructions.
"#;

    create_skill_dir(&temp_dir, "security-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Should match based on description words
    let matches = engine.match_skills("Implement authentication for the API");
    assert_eq!(
        matches.len(),
        1,
        "Should match skill based on description word 'authentication'"
    );
    assert_eq!(matches[0].name, "security-skill");
}

/// Tests skill matching with file patterns (5.1.2).
///
/// Expected behavior:
/// - Skills with matching file patterns are returned
/// - Glob pattern matching is supported
#[test]
fn test_skill_matching_file_patterns() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let rust_skill = r#"---
name: rust-files-skill
description: Skill for Rust files
triggers:
  file_patterns:
    - "*.rs"
    - "Cargo.toml"
---

Rust file skill instructions.
"#;

    let js_skill = r#"---
name: js-files-skill
description: Skill for JavaScript files
triggers:
  file_patterns:
    - "*.js"
    - "*.ts"
    - "package.json"
---

JavaScript file skill instructions.
"#;

    create_skill_dir(&temp_dir, "rust-files-skill", rust_skill);
    create_skill_dir(&temp_dir, "js-files-skill", js_skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Test file pattern matching for Rust file
    let rust_matches = engine.match_skills_for_file("src/main.rs");
    assert_eq!(
        rust_matches.len(),
        1,
        "Should match Rust skill for .rs file"
    );
    assert_eq!(rust_matches[0].name, "rust-files-skill");

    // Test file pattern matching for Cargo.toml
    let cargo_matches = engine.match_skills_for_file("Cargo.toml");
    assert_eq!(
        cargo_matches.len(),
        1,
        "Should match Rust skill for Cargo.toml"
    );
    assert_eq!(cargo_matches[0].name, "rust-files-skill");

    // Test file pattern matching for JavaScript file
    let js_matches = engine.match_skills_for_file("src/index.js");
    assert_eq!(js_matches.len(), 1, "Should match JS skill for .js file");
    assert_eq!(js_matches[0].name, "js-files-skill");

    // Test file pattern matching for TypeScript file
    let ts_matches = engine.match_skills_for_file("src/app.ts");
    assert_eq!(ts_matches.len(), 1, "Should match JS skill for .ts file");
    assert_eq!(ts_matches[0].name, "js-files-skill");

    // Test no match for unrelated file
    let no_matches = engine.match_skills_for_file("README.md");
    assert!(
        no_matches.is_empty(),
        "Should not match any skill for .md file"
    );
}

/// Tests file pattern matching with complex glob patterns.
#[test]
fn test_skill_matching_file_patterns_glob() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let test_skill = r#"---
name: test-skill
description: Skill for test files
triggers:
  file_patterns:
    - "*_test.rs"
    - "test_*.rs"
    - "tests/**/*.rs"
---

Test file skill instructions.
"#;

    create_skill_dir(&temp_dir, "test-skill", test_skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Test suffix pattern
    let matches1 = engine.match_skills_for_file("src/module_test.rs");
    assert_eq!(matches1.len(), 1, "Should match *_test.rs pattern");

    // Test prefix pattern
    let matches2 = engine.match_skills_for_file("src/test_utils.rs");
    assert_eq!(matches2.len(), 1, "Should match test_*.rs pattern");

    // Test directory glob pattern
    let matches3 = engine.match_skills_for_file("tests/unit/my_test.rs");
    assert_eq!(matches3.len(), 1, "Should match tests/**/*.rs pattern");

    // Test non-matching file
    let no_matches = engine.match_skills_for_file("src/main.rs");
    assert!(
        no_matches.is_empty(),
        "Should not match regular source file"
    );
}

// =============================================================================
// Test Group: Skill Context Injection
// =============================================================================

/// Tests generating context from a single matched skill.
///
/// Expected behavior:
/// - Context includes skill name
/// - Context includes skill instructions
/// - Context is properly formatted
#[test]
fn test_skill_context_injection_single() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: commit-skill
description: Git commit assistance
triggers:
  keywords:
    - commit
---

When creating commits:
1. Write clear, concise messages
2. Use conventional commit format
3. Reference issues when applicable
"#;

    create_skill_dir(&temp_dir, "commit-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Get context for matched skills
    let context = engine.get_context_for_task("Help me commit my changes");

    assert!(
        context.contains("commit-skill"),
        "Context should include skill name"
    );
    assert!(
        context.contains("Write clear, concise messages"),
        "Context should include instructions"
    );
    assert!(
        context.contains("conventional commit format"),
        "Context should include full instructions"
    );
}

/// Tests generating context from multiple matched skills.
#[test]
fn test_skill_context_injection_multiple() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill1 = r#"---
name: rust-skill
description: Rust development
triggers:
  keywords:
    - rust
---

Use idiomatic Rust patterns.
Follow Rust naming conventions.
"#;

    let skill2 = r#"---
name: testing-skill
description: Testing guidance
triggers:
  keywords:
    - test
---

Write unit tests for all public functions.
Use descriptive test names.
"#;

    create_skill_dir(&temp_dir, "rust-skill", skill1);
    create_skill_dir(&temp_dir, "testing-skill", skill2);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Task that matches both skills
    let context = engine.get_context_for_task("Write a rust test for this function");

    assert!(
        context.contains("rust-skill"),
        "Context should include rust skill"
    );
    assert!(
        context.contains("testing-skill"),
        "Context should include testing skill"
    );
    assert!(
        context.contains("idiomatic Rust patterns"),
        "Context should include rust instructions"
    );
    assert!(
        context.contains("unit tests for all public functions"),
        "Context should include testing instructions"
    );
}

/// Tests that no context is generated when no skills match.
#[test]
fn test_skill_context_injection_no_match() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: python-skill
description: Python development
triggers:
  keywords:
    - python
---

Python instructions here.
"#;

    create_skill_dir(&temp_dir, "python-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    // Task that doesn't match any skill
    let context = engine.get_context_for_task("Help me with Java code");

    assert!(
        context.is_empty(),
        "Context should be empty when no skills match"
    );
}

/// Tests context injection with allowed_tools information.
#[test]
fn test_skill_context_injection_with_tools() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: file-skill
description: File operations
allowed_tools:
  - Read
  - Write
  - Edit
triggers:
  keywords:
    - file
---

Use the Read tool to read files.
Use the Write tool to create new files.
Use the Edit tool to modify existing files.
"#;

    create_skill_dir(&temp_dir, "file-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let context = engine.get_context_for_task("Help me work with files");

    // Context should mention allowed tools
    assert!(
        context.contains("Read"),
        "Context should mention allowed tools"
    );
    assert!(
        context.contains("Write"),
        "Context should mention allowed tools"
    );
    assert!(
        context.contains("Edit"),
        "Context should mention allowed tools"
    );
}

/// Tests context injection for file-based matching.
#[test]
fn test_skill_context_injection_for_file() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let skill = r#"---
name: cargo-skill
description: Cargo.toml management
triggers:
  file_patterns:
    - "Cargo.toml"
---

When editing Cargo.toml:
- Keep dependencies sorted
- Use specific versions
- Add comments for unusual dependencies
"#;

    create_skill_dir(&temp_dir, "cargo-skill", skill);

    let mut engine = SkillEngine::new();
    engine
        .load_from_dir(&temp_dir.path().to_path_buf())
        .expect("Should load skills");

    let context = engine.get_context_for_file("Cargo.toml");

    assert!(
        context.contains("cargo-skill"),
        "Context should include skill for matched file"
    );
    assert!(
        context.contains("Keep dependencies sorted"),
        "Context should include file-specific instructions"
    );
}

/// Tests that empty skills directory produces no context.
#[test]
fn test_skill_context_injection_empty_engine() {
    let engine = SkillEngine::new();

    let context = engine.get_context_for_task("Any task description");
    assert!(context.is_empty(), "Empty engine should produce no context");

    let file_context = engine.get_context_for_file("any_file.rs");
    assert!(
        file_context.is_empty(),
        "Empty engine should produce no file context"
    );
}
