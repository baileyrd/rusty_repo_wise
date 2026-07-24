use std::path::PathBuf;

fn write_config(dir: &std::path::Path, contents: &str) -> PathBuf {
    let path = dir.join("workspace.toml");
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn load_resolved_resolves_relative_paths_against_the_config_files_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(dir_path.join("member")).unwrap();
    let config_path = write_config(
        &dir_path,
        r#"
            [[repo]]
            name = "member"
            path = "member"
        "#,
    );

    let repos = repowise_workspace::load_resolved(&config_path).unwrap();

    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "member");
    assert_eq!(repos[0].path, dir_path.join("member"));
}

#[test]
fn load_resolved_keeps_absolute_paths_as_given() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let member = dir_path.join("member");
    std::fs::create_dir_all(&member).unwrap();
    let config_path = write_config(
        &dir_path,
        &format!(
            r#"
                [[repo]]
                name = "member"
                path = "{}"
            "#,
            member.display()
        ),
    );

    let repos = repowise_workspace::load_resolved(&config_path).unwrap();

    assert_eq!(repos[0].path, member);
}

#[test]
fn load_resolved_errors_on_a_missing_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.toml");

    assert!(repowise_workspace::load_resolved(&missing).is_err());
}
