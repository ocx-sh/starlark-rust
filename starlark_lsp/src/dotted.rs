/*
 * Copyright 2019 The Starlark in Rust Authors.
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Shared walker for dotted-member access (`a.b.c`) against a host-provided
//! environment `DocModule`.
//!
//! Used by completion, hover, and parameter-name completion to resolve the
//! `DottedDefinition` produced by the AST walker into a concrete `DocItem`.
//! Centralising the walk avoids divergence between the three call sites.
//!
//! v1 scope (matches OCX's `ocx.*` / `expect.*` host globals):
//!
//! - Root identifier must be `IdentifierDefinition::Unresolved` — a name that
//!   was not bound in the current file, so the host environment is consulted.
//!   All other variants (`Location`, `LoadedLocation`, `StringLiteral`, …)
//!   short-circuit to `None`.
//! - Walk descends `DocItem::Module.members` (which holds `DocItem`s) and
//!   `DocItem::Type.members` (which holds `DocMember`s, wrapped back into a
//!   `DocItem::Member` via `Cow::Owned`).
//! - Walk cannot step *into* a `DocItem::Member` — functions and properties are
//!   terminal.
//!
//! v2 follow-ups (out of scope here):
//!
//! - Local-binding aliasing: `o = ocx; o.run` — needs trivial type inference
//!   to forward `o`'s definition through to `ocx`. Today the root binding is
//!   `IdentifierDefinition::Location`, which the helper rejects.
//! - `load()` resolution — disabled in OCX by contract.

use starlark::docs::DocItem;
use starlark::docs::DocMember;
use starlark::docs::DocModule;

use crate::definition::IdentifierDefinition;

/// Walk a dotted-member chain starting from a global identifier and return the
/// resolved leaf `DocItem`.
///
/// `env` is the host environment (typically from `LspContext::get_environment`).
/// `root` is the definition of the leftmost identifier in the dotted chain.
/// `segments` is the full chain *including* the root (`["ocx", "run"]` for
/// `ocx.run`).
///
/// Returns an owned `DocItem` (cloned at each descent step) when the chain
/// resolves; `None` when any step fails. Cloning keeps the implementation
/// straightforward — LSP request volume is too low for the copy cost to
/// matter, and the asymmetry between `DocModule.members` (`DocItem`) and
/// `DocType.members` (`DocMember`) would otherwise force a `Cow` whose owned
/// variant can't safely re-borrow on the next iteration.
pub(crate) fn resolve_dotted_chain(
    env: &DocModule,
    root: &IdentifierDefinition,
    segments: &[String],
) -> Option<DocItem> {
    let name = match root {
        IdentifierDefinition::Unresolved { name, .. } => name.as_str(),
        // v1: only resolve globals supplied via the LSP context environment.
        // Other variants represent file-local bindings, load()ed names, or
        // string literals — none of which the host environment can answer.
        _ => return None,
    };

    // The first segment is the root identifier itself; remaining segments are
    // dot-accessed children.
    let (first, rest) = segments.split_first()?;
    if first.as_str() != name {
        // Defensive: caller passed mismatched root + segments. Bail rather
        // than return a misleading result.
        return None;
    }

    let mut current: DocItem = env.members.get(first.as_str())?.clone();
    for segment in rest {
        current = match &current {
            DocItem::Module(module) => module.members.get(segment.as_str())?.clone(),
            DocItem::Type(typ) => DocItem::Member(typ.members.get(segment.as_str())?.clone()),
            // Cannot descend into a function or property.
            DocItem::Member(_) => return None,
        };
    }
    Some(current)
}

/// Convenience: list the children of the resolved item when it is a `Module`
/// or `Type`. Returns `None` if the chain is unresolved or the leaf is a
/// terminal member (function / property), which has no children to enumerate.
///
/// Pairs (`name`, `DocMember`) are returned for uniform completion-item emit;
/// the type-of-module asymmetry collapses via
/// `DocItem::try_as_member_with_collapsed_object`, mirroring the convention
/// already used by `render_doc_item` for module rendering.
pub(crate) fn list_children(item: &DocItem) -> Option<Vec<(&str, DocMember)>> {
    match item {
        DocItem::Module(module) => Some(
            module
                .members
                .iter()
                .filter_map(|(name, child)| {
                    child
                        .try_as_member_with_collapsed_object()
                        .ok()
                        .map(|member| (name.as_str(), member))
                })
                .collect(),
        ),
        DocItem::Type(typ) => Some(
            typ.members
                .iter()
                .map(|(name, member)| (name.as_str(), member.clone()))
                .collect(),
        ),
        DocItem::Member(_) => None,
    }
}
