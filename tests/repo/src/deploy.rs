use crate::compose::{all_services, re, read, root, service_blocks, set};
use std::collections::BTreeSet;

const REGISTRY: &str = "ghcr.io/amaroke/jellynas-";

fn images_of_the_repo(services: &crate::compose::Services) -> BTreeSet<String> {
    let image = re(r"^    image: (\S+)$");
    services
        .values()
        .flatten()
        .filter_map(|line| image.captures(line))
        .map(|found| found[1].to_string())
        .filter(|image| image.starts_with(REGISTRY))
        .collect()
}

fn builders(services: &crate::compose::Services) -> Vec<String> {
    services
        .iter()
        .filter(|(_, lines)| lines.iter().any(|line| line.starts_with("    build:")))
        .map(|(name, _)| name.clone())
        .collect()
}

#[test]
fn no_service_of_the_stack_builds_on_the_nas() {
    assert_eq!(Vec::<String>::new(), builders(&all_services()));
}

#[test]
fn the_check_tool_builds_nowhere_on_the_nas() {
    let services = service_blocks(&root().join("services/check/compose.yaml"));
    assert_eq!(Vec::<String>::new(), builders(&services));
}

#[test]
fn doco_cd_pulls_a_newer_image_under_the_same_tag() {
    let config = read(&root().join(".doco-cd.yml"));
    assert!(
        config.lines().any(|line| line == "force_image_pull: true"),
        "{config}"
    );
}

#[test]
fn the_ci_builds_every_image_of_the_repo_that_a_service_runs() {
    let mut services = all_services();
    services.extend(service_blocks(&root().join("services/check/compose.yaml")));
    let tag = re(&format!(r#""({}[a-z-]+:main)""#, regex::escape(REGISTRY)));
    let built: BTreeSet<String> = tag
        .captures_iter(&read(&root().join("docker-bake.hcl")))
        .map(|found| found[1].to_string())
        .collect();
    assert_eq!(built, images_of_the_repo(&services));
    assert_eq!(
        set([
            "ghcr.io/amaroke/jellynas-caddy:main",
            "ghcr.io/amaroke/jellynas-check:main",
            "ghcr.io/amaroke/jellynas-dashboard:main",
            "ghcr.io/amaroke/jellynas-discord-posts:main",
            "ghcr.io/amaroke/jellynas-policy:main",
            "ghcr.io/amaroke/jellynas-ratio:main",
            "ghcr.io/amaroke/jellynas-ups:main",
        ]),
        built
    );
}
