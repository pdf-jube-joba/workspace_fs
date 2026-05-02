use anyhow::Result;
use serde::Serialize;

use crate::{
    config::{Policy, PolicyPermissions},
    identity::UserIdentity,
    path::WorkspacePath,
};

#[derive(Debug, Clone, Copy)]
pub enum MethodKind {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PolicyMatchInfo {
    pub index: usize,
    pub path: WorkspacePath,
    pub depth: usize,
    pub permissions: PolicyPermissions,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SelectedPolicyInfo {
    pub index: usize,
    pub path: WorkspacePath,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PolicyInspection {
    pub path: WorkspacePath,
    pub matches: Vec<PolicyMatchInfo>,
    pub selected: Option<SelectedPolicyInfo>,
    pub effective: PolicyPermissions,
}

pub fn resolve_policy(
    method: MethodKind,
    rules: &[Policy],
    path: &WorkspacePath,
    user_identity: &UserIdentity,
) -> Result<Option<bool>> {
    let inspection = inspect_policy_rules(rules, path)?;
    let Some(_) = inspection.selected else {
        return Ok(None);
    };

    let allow = match method {
        MethodKind::Get => &inspection.effective.get,
        MethodKind::Post => &inspection.effective.post,
        MethodKind::Put => &inspection.effective.put,
        MethodKind::Delete => &inspection.effective.delete,
    };

    Ok(Some(
        allow
            .iter()
            .any(|candidate| candidate == user_identity.as_str()),
    ))
}

pub fn inspect_policy_rules(rules: &[Policy], path: &WorkspacePath) -> Result<PolicyInspection> {
    let mut matches = Vec::new();
    let mut selected: Option<(PolicyMatchInfo, String)> = None;
    for (index, rule) in rules.iter().enumerate() {
        if !policy_matches(&rule.path, path) {
            continue;
        }

        let candidate = PolicyMatchInfo {
            index,
            path: rule.path.clone(),
            depth: rule.path.depth(),
            permissions: rule.permissions.clone(),
        };

        match selected {
            Some((ref best, _))
                if best.depth > candidate.depth
                    || (best.depth == candidate.depth && best.index > candidate.index) => {}
            Some((ref best, _)) if best.depth == candidate.depth => {
                selected = Some((candidate.clone(), "later_rule".to_owned()));
            }
            Some((ref best, _)) if best.depth < candidate.depth => {
                selected = Some((candidate.clone(), "more_specific".to_owned()));
            }
            None => {
                selected = Some((candidate.clone(), "first_match".to_owned()));
            }
            _ => {}
        }

        matches.push(candidate);
    }

    let effective = selected
        .as_ref()
        .map(|(selected, _)| selected.permissions.clone())
        .unwrap_or_else(PolicyPermissions::deny_all);
    let selected = selected.map(|(selected, reason)| SelectedPolicyInfo {
        index: selected.index,
        path: selected.path,
        reason,
    });

    Ok(PolicyInspection {
        path: path.clone(),
        matches,
        selected,
        effective,
    })
}

fn policy_matches(rule_path: &WorkspacePath, requested_path: &WorkspacePath) -> bool {
    if rule_path.as_str() == "." {
        return true;
    }

    if rule_path.is_directory() {
        requested_path.starts_with(rule_path)
    } else {
        requested_path.as_str() == rule_path.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Policy;

    fn workspace_path(path: &str) -> WorkspacePath {
        WorkspacePath::from_url(path).unwrap()
    }

    #[test]
    fn more_specific_child_policy_wins() {
        let rules = vec![
            Policy {
                path: WorkspacePath::from_path_str("docs/").unwrap(),
                permissions: PolicyPermissions::deny_all(),
            },
            Policy {
                path: WorkspacePath::from_path_str("docs/public/").unwrap(),
                permissions: PolicyPermissions {
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
                },
            },
        ];

        assert_eq!(
            resolve_policy(
                MethodKind::Get,
                &rules,
                &workspace_path("/docs/public/index.md"),
                &UserIdentity::new("alice_browser")
            )
            .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn equal_specificity_uses_later_rule() {
        let rules = vec![
            Policy {
                path: WorkspacePath::from_path_str("docs/").unwrap(),
                permissions: PolicyPermissions {
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
                },
            },
            Policy {
                path: WorkspacePath::from_path_str("docs/").unwrap(),
                permissions: PolicyPermissions::deny_all(),
            },
        ];

        assert_eq!(
            resolve_policy(
                MethodKind::Get,
                &rules,
                &workspace_path("/docs/a.md"),
                &UserIdentity::new("alice_browser")
            )
            .unwrap(),
            Some(false)
        );
    }

    #[test]
    fn no_matching_policy_denies_by_default() {
        let rules = vec![Policy {
            path: WorkspacePath::from_path_str("docs/").unwrap(),
            permissions: PolicyPermissions {
                get: vec!["alice_browser".into()],
                post: Vec::new(),
                put: Vec::new(),
                delete: Vec::new(),
            },
        }];

        assert_eq!(
            resolve_policy(
                MethodKind::Get,
                &rules,
                &workspace_path("/notes/a.md"),
                &UserIdentity::new("alice_browser")
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn root_policy_applies_recursively() {
        let rules = vec![Policy {
            path: WorkspacePath::from_path_str(".").unwrap(),
            permissions: PolicyPermissions {
                get: vec!["alice_browser".into()],
                post: Vec::new(),
                put: Vec::new(),
                delete: Vec::new(),
            },
        }];

        assert_eq!(
            resolve_policy(
                MethodKind::Get,
                &rules,
                &workspace_path("/folder1/"),
                &UserIdentity::new("alice_browser")
            )
            .unwrap(),
            Some(true)
        );
        assert_eq!(
            resolve_policy(
                MethodKind::Get,
                &rules,
                &workspace_path("/folder1/test1.md"),
                &UserIdentity::new("alice_browser")
            )
            .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn inspection_reports_matches_and_selected_rule() {
        let rules = vec![
            Policy {
                path: WorkspacePath::from_path_str("docs/private/a.md").unwrap(),
                permissions: PolicyPermissions {
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: vec!["alice_cli".into()],
                    delete: Vec::new(),
                },
            },
            Policy {
                path: WorkspacePath::from_path_str("docs/private/").unwrap(),
                permissions: PolicyPermissions::deny_all(),
            },
        ];

        let inspection =
            inspect_policy_rules(&rules, &workspace_path("/docs/private/a.md")).unwrap();

        assert_eq!(inspection.matches.len(), 2);
        let selected = inspection.selected.unwrap();
        assert_eq!(selected.path.as_str(), "docs/private/a.md");
        assert_eq!(selected.reason, "first_match");
        assert_eq!(inspection.effective.get, vec!["alice_browser"]);
    }
}
