//! Guards the workspace's feature-crate dependency shape.
//!
//! Crates that must not depend on each other, directly or transitively, are
//! listed in [`FORBIDDEN_DEPENDENCIES`]: such edges chain large UI crates one
//! after another and serialize the build, badly hurting incremental compile
//! times.
//!
//! This guard previously lived in `tooling/xtask` (`src/workspace.rs`) and moved
//! to `crates/zed/tests` when the xtask task runner was removed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cargo_metadata::{DependencyKind, MetadataCommand};

/// Returns the Cargo workspace.
fn load_workspace() -> cargo_metadata::Metadata {
    MetadataCommand::new()
        .exec()
        .expect("failed to load cargo metadata")
}

/// Crates that must not depend on each other, directly or transitively:
/// such edges chain large UI crates one after another and serialize the
/// build, badly hurting incremental compile times. Dev-dependencies are
/// exempt since they don't affect `cargo build`.
const FORBIDDEN_DEPENDENCIES: &[(&str, &str)] = &[
    ("file_finder", "project_panel"),
    ("open_path_prompt", "project_panel"),
    ("picker", "editor"),
    ("project_panel", "search"),
    ("search", "project_panel"),
];

#[test]
fn no_forbidden_dependencies_between_feature_crates() {
    let workspace = load_workspace();
    let packages = workspace.workspace_packages();
    let member_names = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();

    let mut graph = BTreeMap::new();
    for package in &packages {
        let dependencies = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind != DependencyKind::Development)
            .map(|dependency| dependency.name.as_str())
            .filter(|name| member_names.contains(name))
            .collect::<BTreeSet<_>>();
        graph.insert(package.name.as_str(), dependencies);
    }

    let mut violations = Vec::new();
    for &(from, to) in FORBIDDEN_DEPENDENCIES {
        if let Some(path) = dependency_path(&graph, from, to) {
            violations.push(path.join(" -> "));
        }
    }
    assert_eq!(
        violations,
        Vec::<String>::new(),
        "forbidden dependency paths between sibling feature crates; \
         break the dependency (e.g. by extracting shared code into a lower-level crate) \
         instead of joining these crates into one serial build chain",
    );
}

fn dependency_path<'a>(
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    from: &'a str,
    to: &str,
) -> Option<Vec<&'a str>> {
    let mut parents = BTreeMap::new();
    let mut queue = VecDeque::from([from]);
    while let Some(current) = queue.pop_front() {
        if current == to {
            let mut path = vec![current];
            let mut node = current;
            while let Some(&parent) = parents.get(node) {
                path.push(parent);
                node = parent;
            }
            path.reverse();
            return Some(path);
        }
        for &dependency in graph.get(current).into_iter().flatten() {
            if dependency != from && !parents.contains_key(dependency) {
                parents.insert(dependency, current);
                queue.push_back(dependency);
            }
        }
    }
    None
}
