# Project Report — Suggested Structure

A skeleton for the **≤ 6-page** project report (submit as `g##_report.docx` + `g##_report.pdf`).
Each section notes the marking criterion it targets and a rough page budget. Keep it tight —
prose over screenshots, and put one or two diagrams where they earn their space.

> Reminder: every deliverable must show **Group Number, Student Name(s), Student ID(s)** on the
> cover. And per the spec's AI-use policy, the report must be written in your own words — use this
> only as a structure to fill in, not text to paste.

---

## Cover (not counted toward the 6 pages)
- Title, group number, each member's name + SIT ID, module code (CSC1106), date.

## 1. Introduction & domain analysis  (~0.5 pg) — *Documentation 10%*
- One paragraph: what FerroBank is (a concurrency-safe core-banking web app) and the chosen
  domain (Banking System).
- **Brief literature review** (the spec explicitly wants this): 3–5 sentences on real/OSS core
  banking systems (e.g. Cyclos, Mambu, Apache Fineract) and what you borrowed — double-entry
  thinking, audit trails, OTP/2-step verification, role separation (maker–checker). Use it to
  **justify design decisions** (e.g. "maker–checker in commercial systems motivated our dual
  loan approval and teller account approval").

## 2. System architecture & OOP design  (~1.5 pg) — *Architecture & OOP 15%*
- The layered design: handler → service (trait) → model/SQLx → Postgres; thin handlers, thick services.
- **OOP mapping** (this is where marks are): encapsulation (private service fields, trait-only
  surface), abstraction (six service traits), polymorphism (`dyn Trait` dynamic dispatch +
  generics + enum behaviour), inheritance-substitute (supertraits + composition in `AdminService`).
- Insert the **UML class diagram(s)** from `docs/uml_*.mermaid` (domain model + service architecture).
- Name the four core objects from the tutorial and where each lives (Account, Transfer/Engine, AuditLog).

## 3. Database design  (~0.75 pg) — *Database 10%*
- ER overview: `users, accounts, transfers, loans, loan_approvals, repayments, audit_log`.
- Normalisation + integrity: FKs, enums as Postgres types, `NUMERIC(18,2)` for money, CHECK
  constraints (positive amounts, non-negative balance), the **customer-only-accounts trigger**.
- Why Postgres (concurrency, `FOR UPDATE`, transactions) — tie back to the literature review.

## 4. Core features & business logic  (~1.25 pg) — *Backend & business logic 15%*
- **Concurrency-safe transfer engine** (the headline): two-step create→confirm, OTP (argon2-hashed),
  Mutex rate-limit, `SELECT … FOR UPDATE` row locks, lock ordering, **explicit rollback**, audit.
- Fraud detection rules (rejected / large / structuring / velocity).
- Loans: simple-interest model, **dual approval (teller + admin)**, repayment that debits an account.
- Accounts: teller-approved opening (`pending → active`), freeze/adjust.
- RBAC: customer / teller / admin, `RequireRole` guard, `/admin` vs `/staff` scopes.

## 5. Frontend & server-side rendering  (~0.5 pg) — *Frontend/SSR 10%*
- Askama templates, shared `layout.html`, partials, reusable status badges.
- UX touches: light/dark theme, count-up animation, client-side search, inline form errors.

## 6. Individual extended features  (~1 pg total) — *Individual 40%*
- One short subsection **per member**: the feature they own, why it's non-trivial, and the key
  technical decision. Map to the modules (Auth, Accounts, Transfers/concurrency, Loans, Admin/Staff).
- Each member should be able to defend their part in the demo Q&A (the 10% "individual understanding").

## 7. Testing, limitations & future work  (~0.25 pg)
- How you verified correctness (e.g. the double-spend concurrency test from `DEMO_SCENARIOS.md`).
- Honest limitations: in-process Mutex (single instance), simple-interest model, OTP shown on screen.
- Future work: account-subtype trait polymorphism, distributed rate limiting, statements/PDF.

## 8. Conclusion  (~0.25 pg)
- What was delivered against the spec; one line per member's contribution.

## References
- Literature sources cited in §1 (Cyclos / Mambu / Fineract docs, Rust/Actix/SQLx docs, etc.).

---

### Page-budget cheat-sheet
| Section | Pages | Criterion |
|---|---|---|
| 1 Intro + lit review | 0.5 | Documentation |
| 2 Architecture + OOP + UML | 1.5 | Architecture/OOP (15%) |
| 3 Database | 0.75 | Database (10%) |
| 4 Features/business logic | 1.25 | Backend (15%) |
| 5 Frontend/SSR | 0.5 | Frontend (10%) |
| 6 Individual features | 1.0 | Individual (40%) |
| 7–8 Testing/limits/conclusion | 0.5 | Documentation |
| **Total** | **~6** | |
