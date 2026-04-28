//! Route handlers, grouped by surface area:
//!
//! - [`core`]: the main pipeline endpoints (`/remember`, `/context`, `/ask`)
//!   and REST resources (`/entities`, `/edges`).
//! - [`observability`]: `/health`, `/metrics`, `/events`.
//! - [`graphs`]: graph-data ops (`/seed`, `/admin/backup`, `/admin/restore`,
//!   `/graphs`, `/graphs/drop/{name}`).
//! - [`admin`]: user and API-key management plus `/admin/audit`.

pub(super) mod admin;
pub(super) mod core;
pub(super) mod graphs;
pub(super) mod observability;
