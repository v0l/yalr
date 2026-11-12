use super::Database;
use std::collections::{HashMap, HashSet};

pub const MAX_NESTING_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedLinkError {
    SelfReference,
    Cycle,
    TooDeep,
}

impl std::fmt::Display for NestedLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NestedLinkError::SelfReference => write!(f, "A routing config cannot contain itself"),
            NestedLinkError::Cycle => write!(f, "Adding this routing config would create a cycle"),
            NestedLinkError::TooDeep => {
                write!(f, "Nested routing configs may be at most {MAX_NESTING_DEPTH} levels deep")
            }
        }
    }
}

type Edges = HashMap<i64, Vec<i64>>;

fn reaches(edges: &Edges, from: i64, target: i64) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if seen.insert(node) {
            stack.extend(edges.get(&node).into_iter().flatten().copied());
        }
    }
    false
}

fn levels_below(edges: &Edges, node: i64, path: &mut Vec<i64>) -> usize {
    if path.contains(&node) {
        return MAX_NESTING_DEPTH + 1;
    }
    path.push(node);
    let deepest = edges
        .get(&node)
        .into_iter()
        .flatten()
        .map(|child| levels_below(edges, *child, path))
        .max()
        .unwrap_or(0);
    path.pop();
    1 + deepest
}

fn levels_above(parents: &Edges, node: i64, path: &mut Vec<i64>) -> usize {
    if path.contains(&node) {
        return MAX_NESTING_DEPTH + 1;
    }
    path.push(node);
    let highest = parents
        .get(&node)
        .into_iter()
        .flatten()
        .map(|parent| levels_above(parents, *parent, path))
        .max()
        .unwrap_or(0);
    path.pop();
    1 + highest
}

pub fn check_link(edges: &Edges, parent: i64, child: i64) -> Result<(), NestedLinkError> {
    if parent == child {
        return Err(NestedLinkError::SelfReference);
    }
    if reaches(edges, child, parent) {
        return Err(NestedLinkError::Cycle);
    }
    let mut parents: Edges = HashMap::new();
    for (from, tos) in edges {
        for to in tos {
            parents.entry(*to).or_default().push(*from);
        }
    }
    let depth = levels_above(&parents, parent, &mut Vec::new()) + levels_below(edges, child, &mut Vec::new());
    if depth > MAX_NESTING_DEPTH {
        return Err(NestedLinkError::TooDeep);
    }
    Ok(())
}

impl Database {
    async fn nesting_edges(&self) -> Result<Edges, sqlx::Error> {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT routing_config_id, child_config_id FROM routing_config_providers \
             WHERE child_config_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut edges: Edges = HashMap::new();
        for (parent, child) in rows {
            edges.entry(parent).or_default().push(child);
        }
        Ok(edges)
    }

    pub async fn validate_nested_link(&self, parent: i64, child: i64) -> Result<Result<(), NestedLinkError>, sqlx::Error> {
        Ok(check_link(&self.nesting_edges().await?, parent, child))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(list: &[(i64, i64)]) -> Edges {
        let mut out: Edges = HashMap::new();
        for (a, b) in list {
            out.entry(*a).or_default().push(*b);
        }
        out
    }

    #[test]
    fn self_reference_and_cycles_are_refused() {
        assert_eq!(check_link(&edges(&[]), 1, 1), Err(NestedLinkError::SelfReference));
        assert_eq!(check_link(&edges(&[(1, 2)]), 2, 1), Err(NestedLinkError::Cycle));
        assert_eq!(check_link(&edges(&[(1, 2), (2, 3)]), 3, 1), Err(NestedLinkError::Cycle));
        assert_eq!(check_link(&edges(&[(1, 2)]), 1, 3), Ok(()));
    }

    #[test]
    fn chains_longer_than_the_cap_are_refused() {
        let chain = edges(&[(1, 2), (2, 3), (3, 4)]);
        assert_eq!(check_link(&chain, 4, 5), Err(NestedLinkError::TooDeep));
        assert_eq!(check_link(&chain, 0, 1), Err(NestedLinkError::TooDeep));
        assert_eq!(check_link(&edges(&[(1, 2), (2, 3)]), 3, 4), Ok(()));
    }
}
