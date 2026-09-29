//! Cycle and expansion gates for named-type graphs.
//!
//! Arrow `Struct`/`List` is a finite tree. Cyclic type graphs cannot be stored
//! columns or materialized procedure values. Checks are iterative (no recursive
//! Rust calls unbounded by user SQL) and keyed by opaque [`TypeId`].

use std::collections::{HashMap, HashSet};

use super::LogicalTypeRef;
use crate::models::TypeId;

/// Default maximum nesting of named types (including list wrappers).
pub const MAX_TYPE_GRAPH_DEPTH: usize = 32;
/// Default maximum resolved Arrow field count across a type expansion.
pub const MAX_RESOLVED_ARROW_FIELDS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeGraphError {
    pub message: String,
}

impl TypeGraphError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TypeGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TypeGraphError {}

/// One catalog type's named-type children and live field count.
#[derive(Debug, Clone, Default)]
pub struct TypeGraphNode {
    pub children:    Vec<TypeId>,
    pub field_count: usize,
}

impl TypeGraphNode {
    pub fn from_type_refs<'a>(fields: impl IntoIterator<Item = &'a LogicalTypeRef>) -> Self {
        let mut children = Vec::new();
        let mut field_count = 0usize;
        for type_ref in fields {
            field_count += 1;
            children.extend(type_ref.referenced_type_ids());
        }
        Self {
            children,
            field_count,
        }
    }
}

/// Reject direct/indirect cycles, including through lists, and cap depth/fields.
pub fn assert_finite_type_graph(
    root: &TypeId,
    nodes: &HashMap<TypeId, TypeGraphNode>,
) -> Result<(), TypeGraphError> {
    assert_finite_type_graph_with_limits(
        root,
        nodes,
        MAX_TYPE_GRAPH_DEPTH,
        MAX_RESOLVED_ARROW_FIELDS,
    )
}

enum Frame {
    Enter(TypeId, usize),
    Exit(TypeId),
}

pub fn assert_finite_type_graph_with_limits(
    root: &TypeId,
    nodes: &HashMap<TypeId, TypeGraphNode>,
    max_depth: usize,
    max_fields: usize,
) -> Result<(), TypeGraphError> {
    let mut on_path: HashSet<TypeId> = HashSet::new();
    let mut stack = vec![Frame::Enter(root.clone(), 0)];
    let mut expanded_fields = 0usize;

    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Enter(current, depth) => {
                if depth > max_depth {
                    return Err(TypeGraphError::new(format!(
                        "named type {root} exceeds maximum nesting depth {max_depth}"
                    )));
                }
                if !on_path.insert(current.clone()) {
                    return Err(TypeGraphError::new(format!(
                        "cyclic named type graph involving {current}"
                    )));
                }
                stack.push(Frame::Exit(current.clone()));
                let Some(node) = nodes.get(&current) else {
                    continue;
                };
                expanded_fields = expanded_fields.saturating_add(node.field_count);
                if expanded_fields > max_fields {
                    return Err(TypeGraphError::new(format!(
                        "named type {root} expands to more than {max_fields} Arrow fields"
                    )));
                }
                for child in node.children.iter().rev() {
                    stack.push(Frame::Enter(child.clone(), depth + 1));
                }
            },
            Frame::Exit(current) => {
                on_path.remove(&current);
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NamespaceId;

    fn id(name: &str) -> TypeId {
        TypeId::from_parts(Some(&NamespaceId::new("chat")), name)
    }

    #[test]
    fn rejects_direct_cycle() {
        let a = id("a");
        let mut nodes = HashMap::new();
        nodes.insert(
            a.clone(),
            TypeGraphNode {
                children:    vec![a.clone()],
                field_count: 1,
            },
        );
        assert!(assert_finite_type_graph(&a, &nodes).is_err());
    }

    #[test]
    fn rejects_indirect_cycle() {
        let a = id("a");
        let b = id("b");
        let mut nodes = HashMap::new();
        nodes.insert(
            a.clone(),
            TypeGraphNode {
                children:    vec![b.clone()],
                field_count: 1,
            },
        );
        nodes.insert(
            b.clone(),
            TypeGraphNode {
                children:    vec![a.clone()],
                field_count: 1,
            },
        );
        assert!(assert_finite_type_graph(&a, &nodes).is_err());
    }

    #[test]
    fn allows_diamond() {
        let user = id("user");
        let address = id("address");
        let mut nodes = HashMap::new();
        nodes.insert(
            user.clone(),
            TypeGraphNode {
                children:    vec![address.clone(), address.clone()],
                field_count: 2,
            },
        );
        nodes.insert(
            address,
            TypeGraphNode {
                children:    vec![],
                field_count: 2,
            },
        );
        assert!(assert_finite_type_graph(&user, &nodes).is_ok());
    }

    #[test]
    fn caps_depth() {
        let a = id("a");
        let b = id("b");
        let mut nodes = HashMap::new();
        nodes.insert(
            a.clone(),
            TypeGraphNode {
                children:    vec![b.clone()],
                field_count: 1,
            },
        );
        nodes.insert(
            b,
            TypeGraphNode {
                children:    vec![],
                field_count: 1,
            },
        );
        assert!(assert_finite_type_graph_with_limits(&a, &nodes, 0, 1024).is_err());
    }

    #[test]
    fn rejects_cycle_through_list_child() {
        let a = id("node");
        let list = LogicalTypeRef::list(LogicalTypeRef::named(a.clone()), true);
        let mut nodes = HashMap::new();
        nodes.insert(a.clone(), TypeGraphNode::from_type_refs(std::slice::from_ref(&list)));
        assert!(assert_finite_type_graph(&a, &nodes).is_err());
    }
}
