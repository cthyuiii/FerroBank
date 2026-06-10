# FerroBank — Project Proposal (Formative Assessment Draft)

> **Group Number:** g## · **Members:** Name 1 (SIT ID), Name 2 (SIT ID), Name 3 (SIT ID), Name 4 (SIT ID), Name 5 (SIT ID)
>
> CSC1106 Web Programming — Banking System domain (spec v1.2.2). Max 2 pages.
>
> **Planning draft.** Rewrite every section in your own words, verify the
> references, and fill in the real member allocations before any submission.

## 1. Project Direction and Business Workflow Planning

**Selected domain:** Banking System. We propose **FerroBank**, a server-side-rendered core
banking platform built with Rust and Actix Web, backed by PostgreSQL. The system models the
day-to-day operations of a retail bank across three user roles — **customer**, **teller**, and
**admin** — each with distinct workflows and access rights enforced by session-based
authentication and role-guard middleware.

**Core business workflows:**

1. **Account lifecycle with maker–checker control.** Customers self-open savings or checking
   accounts, which enter a *pending* state until a teller or admin approves them — mirroring
   real KYC/onboarding controls. Staff can freeze, unfreeze, and close accounts; closure is
   refused while a balance remains.
2. **Two-step money transfer with OTP confirmation.** A transfer is created as a *pending*
   intent (no money moves), protected by a simulated one-time password. Confirmation verifies
   the OTP and executes the movement atomically. Rejections (insufficient funds, frozen
   account, bad OTP) are recorded with human-readable reasons.
3. **Loan origination with dual approval.** Customers apply for simple-interest loans; an
   application only becomes approved after **both** a teller and an admin sign off (the
   four-eyes principle used in real credit operations). Repayments debit a chosen funding
   account in the same database transaction that records the payment.
4. **Supervision and compliance.** An admin dashboard aggregates deposits, loan portfolio
   exposure, pending work queues, fraud signals, and a searchable append-only audit log.

## 2. Feature Scope and Complexity Planning

The technical centrepiece, as the spec requires, is a **concurrency-safe money transfer
engine** with two cooperating layers of protection:

- **Application layer:** an `Arc`-shared, `tokio::sync::Mutex`-guarded rate-limit map caps
  each account at 5 transfer attempts per minute before any database work begins.
- **Database layer:** confirmation runs in a single SQL transaction that locks both account
  rows with `SELECT … FOR UPDATE` (always in ascending id order to eliminate deadlocks),
  re-validates balances and statuses under the lock, and rolls back explicitly on any
  failure — preventing lost updates, double-spending, and negative balances. Money is stored
  as `NUMERIC`/`rust_decimal`, never floating point.

**OOP design:** behaviour is defined by service traits (`AuthService`, `AccountService`,
`TransferService`, `LoanService`, `AuditService`, `AdminService`) with PostgreSQL-backed
implementations consumed as trait objects (`Arc<dyn Trait>`), giving encapsulation,
abstraction, and runtime polymorphism; domain entities are structs with `impl` blocks.

**Individual extended features (one per member):**

| Member | Extended feature |
|---|---|
| M1 (Platform Lead) | Admin dashboard with rule-based fraud detection (large transfers ≥ $10k, structuring just under $10k, 24-hour velocity), RBAC middleware |
| M2 | Authentication: argon2id password hashing, cookie sessions, role-based redirects |
| M3 | Account approval workflow + staff balance adjustments with row-locked invariants |
| M4 | OTP-protected transfer engine (argon2-hashed OTPs), append-only audit logging |
| M5 | Dual-approval loan workflow + transactional repayments against funding accounts |

## 3. Literature Review and Background Study

Our design choices are grounded in established banking-systems practice. **Race conditions in
funds transfer** are the canonical example of lost-update anomalies; the standard remedies are
pessimistic row locking and ACID transactions (Silberschatz et al., *Database System Concepts*,
7th ed., ch. 17–18), which we adopt via PostgreSQL `FOR UPDATE` and explicit rollback.
**Deadlock avoidance by ordered lock acquisition** follows the resource-ordering technique
described in operating-systems literature (Tanenbaum & Bos, *Modern Operating Systems*).
**Password and OTP storage** follows the OWASP Password Storage Cheat Sheet recommendation of
argon2id. The **fraud rules** mirror real AML practice: the US Bank Secrecy Act sets a $10,000
currency-reporting threshold, and FinCEN documents *structuring* (splitting amounts to stay
just under it) as a primary red flag — our $9k–$10k and velocity rules are simplified versions.
**Dual control (four-eyes)** for credit approval is a standard internal-control requirement in
banking governance (Basel Committee, *Internal audit function in banks*). Finally, a survey of
open-source cores such as Apache Fineract informed our module split (customers, accounts,
ledger-like transfers, loans, audit) and our choice of an append-only audit trail.

**Stack justification:** Rust + Actix Web give memory-safe, thread-safe concurrency suited to
a transfer engine; Askama provides compile-time-checked SSR templates; PostgreSQL provides the
transactional guarantees the domain demands. The system will be delivered with Docker Compose,
seeded demo data, and UML domain/service diagrams.
