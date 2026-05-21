//! Actix route handlers, one file per module owner.
//!
//! Every module exports a `pub fn routes(cfg: &mut web::ServiceConfig)` function.
//! Only `src/routes.rs` calls those — module owners never touch each other's routes.

pub mod accounts;
pub mod admin;
pub mod auth;
pub mod home;
pub mod loans;
pub mod transfers;

/// Polite 501 stand-in used by stub handlers until module owners replace them.
/// Delete this helper and all of its callsites once every module is implemented.
pub(crate) fn coming_soon(module: &str, owner: &str) -> actix_web::HttpResponse {
    let body = format!(
        r#"<!doctype html>
<html lang="en"><head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{module} · FerroBank</title>
  <script src="https://cdn.tailwindcss.com"></script>
</head>
<body class="bg-stone-50 text-stone-900 antialiased">
  <div class="max-w-xl mx-auto px-6 py-20">
    <div class="text-6xl">🚧</div>
    <h1 class="text-3xl font-semibold mt-4">{module}</h1>
    <p class="mt-3 text-stone-600">This module is being built by <span class="font-mono font-semibold">{owner}</span>.</p>
    <p class="mt-2 text-stone-600">See <code class="bg-stone-200 px-1.5 rounded">TEAM_CHARTER.md</code> for the assignment.</p>
    <a href="/" class="inline-block mt-6 text-amber-700 hover:underline">← Home</a>
  </div>
</body></html>"#
    );
    actix_web::HttpResponse::NotImplemented()
        .content_type("text/html; charset=utf-8")
        .body(body)
}
