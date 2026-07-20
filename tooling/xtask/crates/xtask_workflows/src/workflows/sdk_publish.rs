use gh_workflow::{Concurrency, Event, Expression, Job, Push, Run, Step, Use, Workflow};

use crate::workflows::{runners, steps, vars};

/// Build the workflow.
pub fn sdk_publish() -> Workflow {
    Workflow::new("SDK Publish")
        .on(Event::default().push(
            Push::default()
                .add_branch("main")
                .add_path(xtask_paths::repo_glob!(".github/workflows/sdk-publish.yml"))
                .add_path(xtask_paths::repo_glob!("packages/sdk/**")),
        ))
        // Never cancel a publish mid-flight; serialize runs of this workflow.
        .concurrency(
            Concurrency::new(Expression::new("${{ github.workflow }}")).cancel_in_progress(false),
        )
        .add_job("publish", publish())
}

fn publish() -> Job {
    Job::default()
        .name("Publish to npm")
        .runs_on(runners::Runner::Small.to_string())
        .add_step(steps::checkout(false, false))
        .add_step(steps::setup_bun())
        .add_step(setup_node())
        .add_step(install_deps())
        .add_step(version_guard())
        .add_step(build())
        .add_step(publish_step())
}

/// `registry-url` makes setup-node write the scoped `~/.npmrc` that `npm publish`
/// reads via `NODE_AUTH_TOKEN`.
fn setup_node() -> Step<Use> {
    Step::new("Setup Node")
        .uses("actions", "setup-node", "v4")
        .add_with(("node-version", "22"))
        .add_with(("registry-url", "https://registry.npmjs.org"))
        .add_with(("always-auth", true))
}

fn install_deps() -> Step<Run> {
    Step::new("Install dependencies")
        .run("bun install --frozen-lockfile")
        .working_directory(xtask_paths::repo_dir!("packages/sdk"))
}

/// Publish only when the version in `package.json` isn't already on npm. This is
/// idempotent — re-runs and merges that don't bump the version are no-ops.
fn version_guard() -> Step<Run> {
    Step::new("Check whether version is already published")
        .run(indoc::indoc! {r#"
            set -euo pipefail
            version="$(npm pkg get version | tr -d '"')"
            echo "version=$version" >> "$GITHUB_OUTPUT"
            if npm view "@macro/sdk@$version" version >/dev/null 2>&1; then
              echo "should_publish=false" >> "$GITHUB_OUTPUT"
              echo "@macro/sdk@$version is already published; skipping."
            else
              echo "should_publish=true" >> "$GITHUB_OUTPUT"
              echo "@macro/sdk@$version is not on npm; will publish."
            fi
        "#})
        .id("guard")
        .working_directory(xtask_paths::repo_dir!("packages/sdk"))
}

fn build() -> Step<Run> {
    Step::new("Build")
        .run("bun run build")
        .if_condition(Expression::new("steps.guard.outputs.should_publish == 'true'"))
        .working_directory(xtask_paths::repo_dir!("packages/sdk"))
}

fn publish_step() -> Step<Run> {
    Step::new("Publish to npm")
        .run("npm publish --access public")
        .if_condition(Expression::new("steps.guard.outputs.should_publish == 'true'"))
        .working_directory(xtask_paths::repo_dir!("packages/sdk"))
        .add_env(("NODE_AUTH_TOKEN", vars::NPM_TOKEN))
}
