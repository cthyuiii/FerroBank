# FerroBank - Project Proposal

> **Group Number:** g## · **Members:** Name 1 (SIT ID), Name 2 (SIT ID), Name 3 (SIT ID), Name 4 (SIT ID), Name 5 (SIT ID)
>
> CSC1106 Web Programming - Banking System domain. Sections mirror
> [REPORT_OUTLINE.md](./REPORT_OUTLINE.md) so the proposal grows into the
> report without restructuring.
>
> **Planning draft.** Rewrite every section in your own words, verify the
> references, and fill in the real member allocations before any submission.

## 1. Introduction & domain analysis

We propose **FerroBank**, a server-side-rendered retail-banking platform built
with Rust, Actix Web, and PostgreSQL. The system models a bank's day-to-day
operations across three roles - customer, teller, and admin - with realistic
controls drawn from industry practice: maker-checker account approval,
four-eyes loan decisions, out-of-band one-time codes, transfer limits with
cooling-off windows, fraud holds with identity review, device tracking, and an
append-only audit trail. The spec's "concurrency-safe money transfer engine"
is treated as the technical heart, and every supporting workflow exists to
make that engine demonstrably safe, accountable, and controlled.

## 2. System architecture & OOP design

A strict layered architecture: thin Actix handlers (parse, call, render) over
thick service traits (`AuthService`, `AccountService`, `TransferService`,
`LoanService`, `ActionOtpService`, `AuditService`, `AdminService`) consumed as
`Arc<dyn Trait>` - encapsulation (private pools and channels), abstraction
(traits as interfaces), and runtime polymorphism (Postgres impls today, mocks
or alternatives tomorrow). The `OtpChannel` trait with `TelegramOtp` and
`ScreenOtp` implementations shows substitutability concretely: the transfer
engine never knows which channel it holds. Cross-cutting concerns live in
middleware: a session guard, role-based access (admin superuser semantics),
and an `ActivityGuard` enforcing a 5-minute inactivity TTL on database time
while logging every authenticated request. Diagrams:
[uml_service_architecture.mermaid](./uml_service_architecture.mermaid),
[FLOWS.md](./FLOWS.md).

## 3. Database design

PostgreSQL 16 with one migration per module: users (structured names, NRIC,
Telegram linkage, activity timestamp), login_sessions (device tracking),
accounts (balances, per-transfer limits, hold windows), limit_changes and
adjustment_requests (delayed and dual-controlled mutations), transfers with
transfer_reviews (the on-hold pipeline) and hashed OTPs, loans with
loan_approvals (role-keyed dual control), repayments, action_otps (the
generalized OTP guard), audit_log, and notifications. Money is `NUMERIC(18,2)`
- never floating point. Integrity is enforced in the schema itself: CHECK
constraints, UNIQUE(loan, role), and triggers that block staff-owned accounts
and same-owner transfers. ER model:
[er_diagram.mermaid](./er_diagram.mermaid).

## 4. Core features & business logic

The transfer engine is two-phase: create validates ownership, funds, limits,
and rate (a `tokio::sync::Mutex` window) and parks an argon2-hashed OTP;
confirm re-validates everything under `SELECT ... FOR UPDATE` row locks taken
in ascending id order, then debits, credits, and finalizes in one transaction
with explicit rollback. Fraud rules evaluated under the same locks divert
matches to an on-hold state where the customer states a purpose and proves
identity (NRIC) and staff release or deny. Supporting workflows: maker-checker
account opening, dual-approval loans that disburse principal and schedule
monthly due dates, repayments that debit real funding accounts, automatic
freezes after repeated overdraft attempts, and a graceful-shutdown state
snapshot.

## Security measures implemented

(Folds into report sections 4 and 7; gathered here so the controls can be
assessed as one architecture.)

**Authentication & identity.** argon2id password hashing with unique salts;
identical login errors for wrong email vs wrong password (no user
enumeration); mandatory out-of-band channel (Telegram) for every customer;
risk-based step-up login - a first-seen browser or network must also present
a one-time code before any session exists, and three wrong codes block that
origin from the account for 24 hours even with the correct password; per-login
device tracking (browser family, IP, first-seen flags) with automatic owner
alerts; hardened unlink - the bot's /unlink takes effect only after 24 hours
with warnings on every channel, cancellable solely from a password-backed
website session.

**Session security.** Cookie keys derived from the secret plus a per-boot
nonce, so sessions cannot outlive the server process; a 5-minute inactivity
TTL enforced server-side on database time; every authenticated request logged
with user id, method, and path.

**One-time-code discipline.** A code gates every sensitive action - transfers,
account opening, loan applications, limit changes, profile changes, unlinking,
and risky logins. All codes are argon2-hashed at rest, single-use, expire
after 10 minutes, and carry a uniform 3-strike budget: the third wrong code
cancels the pending action (and, for logins, blocks the origin).

**Transaction integrity.** `SELECT ... FOR UPDATE` row locks in ascending id
order inside single transactions with explicit rollback; money as
`NUMERIC(18,2)`; database-level CHECK constraints and triggers (non-negative
balances, customer-only accounts, no same-owner transfers) backing up every
service-level rule; funds verified before anything posts; per-transfer limits
whose increases are held 12 hours; fraud rules that park matches on hold for
NRIC-verified staff review instead of completing; automatic account freeze
after three overdraft attempts in 24 hours; dual control (two distinct staff
roles) on loan approvals and on balance adjustments above $1,000.

**Platform & accountability.** All SQL parameterized through SQLx (no string
interpolation anywhere); Askama auto-escaping for output (XSS); Rust's memory
safety; an append-only audit log behind every state change; per-user activity
views for staff; graceful-shutdown state snapshots. Known limitations (no
TLS in the dev deployment, plaintext NRIC at rest, unthrottled password
attempts from known origins) are stated in section 7 rather than hidden.

## 5. Frontend & server-side rendering

Askama templates (compile-time checked) under one shared layout with reusable
partials; Tailwind via CDN with a custom dark mode; HTMX-progressive forms.
Light JavaScript is used only where SSR cannot reach: confirmation
modals, toast notifications polled from the server, local-time rendering of
held limit changes, and per-item status polls that auto-refresh pending pages
the moment staff act.

## 6. Individual extended features

One per member, each independently demonstrable and verifiable in the audit
log: (M1) admin dashboard with rule-based fraud detection, the held-transfer
review queue, and the per-user activity view; (M2) the identity stack -
mandatory Telegram linking, out-of-band OTP delivery, bot commands, login
device tracking with new-device/new-network alerts; (M3) account lifecycle
controls - approval workflow, transfer limits with held increases, and
dual-control balance adjustments; (M4) the concurrency-safe transfer engine
itself plus the race-demo lab and integration tests proving no overdraft, no
loss, and no double spend; (M5) the loan lifecycle - dual approval,
disbursement to a chosen account, due-date scheduling, and repayments.

## 7. Testing, limitations & future work

Unit tests cover the interest mathematics; DB-gated integration tests prove
the engine's invariants under genuine concurrency (simultaneous transfers, a
replayed double confirm) and the shutdown snapshot. Known limitations stated
honestly: cookie sessions rather than server-side session storage,
simple-interest loans rather than amortization, single-node deployment, and
Telegram standing in for SMS. Future work: an `SmsOtp` implementation of
`OtpChannel`, statement export, and scheduled (rather than lazy) maturation of
limit changes.

## 8. Conclusion

FerroBank demonstrates that the spec's banking domain can be implemented with
real engineering substance: a provably safe money engine, OOP that earns its
keep through trait-based polymorphism, a schema that enforces its own
invariants, and controls that mirror how banks actually operate.

## References

To verify and format before submission: PostgreSQL documentation on explicit
locking; OWASP Password Storage Cheat Sheet (argon2id); FinCEN guidance on
structuring; Basel Committee internal-control principles (four-eyes); Actix
Web and SQLx documentation; Apache Fineract as an open-source comparison
system.
