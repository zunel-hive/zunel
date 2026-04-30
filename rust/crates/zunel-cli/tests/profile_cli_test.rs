use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn profile_use_show_list_and_remove_named_profile() {
    let home = tempfile::tempdir().unwrap();
    let default_home = home.path().join(".zunel");
    let dev_home = default_home.join("profiles").join("dev");
    fs::create_dir_all(&dev_home).unwrap();

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "use", "dev"])
        .assert()
        .success()
        .stdout(contains("Active profile set to dev"));

    assert_eq!(
        fs::read_to_string(default_home.join("active_profile")).unwrap(),
        "dev\n"
    );

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "show"])
        .assert()
        .success()
        .stdout(contains("profile: dev"))
        .stdout(contains(format!("home: {}", dev_home.display())));

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(contains("default"))
        .stdout(contains("dev"));

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "use", "default"])
        .assert()
        .success()
        .stdout(contains("Cleared sticky profile"));

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "rm", "dev", "--force"])
        .assert()
        .success()
        .stdout(contains("Removed"));

    assert!(!dev_home.exists());
}

#[test]
fn profile_rejects_unsafe_names() {
    let home = tempfile::tempdir().unwrap();

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["profile", "use", "../bad"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("Invalid profile name"));
}

#[test]
fn global_profile_flag_routes_command_to_named_home() {
    let home = tempfile::tempdir().unwrap();
    let dev_home = home.path().join(".zunel").join("profiles").join("dev");

    Command::cargo_bin("zunel")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("ZUNEL_HOME")
        .args(["--profile", "dev", "onboard"])
        .assert()
        .success()
        .stdout(contains(format!("onboarded: {}", dev_home.display())));

    assert!(dev_home.join("config.json").exists());
    assert!(dev_home.join("workspace").is_dir());
}
