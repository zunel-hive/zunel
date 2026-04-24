use std::sync::OnceLock;

use minijinja::{Environment, Value};

static IDENTITY: &str = include_str!("../templates/identity.md");
static PLATFORM_POLICY: &str = include_str!("../templates/platform_policy.md");
static SKILLS_SECTION: &str = include_str!("../templates/skills_section.md");
static MAX_ITERATIONS_MESSAGE: &str = include_str!("../templates/max_iterations_message.md");

fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut e = Environment::new();
        e.add_template("identity", IDENTITY)
            .expect("identity template compiles");
        e.add_template("platform_policy", PLATFORM_POLICY)
            .expect("policy template compiles");
        e.add_template("skills_section", SKILLS_SECTION)
            .expect("skills template compiles");
        e.add_template("max_iterations_message", MAX_ITERATIONS_MESSAGE)
            .expect("max iterations template compiles");
        e
    })
}

pub fn render_identity(
    workspace: &str,
    runtime: &str,
    channel: Option<&str>,
) -> Result<String, minijinja::Error> {
    let tmpl = env().get_template("identity")?;
    tmpl.render(Value::from_serialize(serde_json::json!({
        "workspace": workspace,
        "runtime": runtime,
        "channel": channel,
    })))
}

pub fn render_platform_policy() -> Result<String, minijinja::Error> {
    env().get_template("platform_policy")?.render(())
}

pub fn render_skills_section(skills_summary: &str) -> Result<String, minijinja::Error> {
    let tmpl = env().get_template("skills_section")?;
    tmpl.render(Value::from_serialize(serde_json::json!({
        "skills_summary": skills_summary,
    })))
}

pub fn render_max_iterations_message(tools_used: &[String]) -> Result<String, minijinja::Error> {
    let tmpl = env().get_template("max_iterations_message")?;
    tmpl.render(Value::from_serialize(serde_json::json!({
        "tools_used": tools_used,
    })))
}
