use tempfile::tempdir;

use zunel_tools::path_policy::PathPolicy;

#[test]
fn absolute_under_workspace_is_allowed() {
    let ws = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path());
    let target = ws.path().join("a.txt");
    assert!(policy.check(&target).is_ok());
}

#[test]
fn absolute_outside_workspace_is_denied_when_restricted() {
    let ws = tempdir().unwrap();
    let other = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path());
    let err = policy.check(&other.path().join("x.txt")).unwrap_err();
    assert!(err.to_string().contains("outside workspace"), "{err}");
}

#[test]
fn unrestricted_allows_any_path() {
    let other = tempdir().unwrap();
    let policy = PathPolicy::unrestricted();
    assert!(policy.check(&other.path().join("x.txt")).is_ok());
}

#[test]
fn media_dir_escape_hatch_allows_subpaths() {
    let ws = tempdir().unwrap();
    let media = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path()).with_media_dir(media.path());
    assert!(policy.check(&media.path().join("file.png")).is_ok());
    let err = policy
        .check(&media.path().parent().unwrap().join("elsewhere"))
        .unwrap_err();
    assert!(err.to_string().contains("outside workspace"), "{err}");
}
