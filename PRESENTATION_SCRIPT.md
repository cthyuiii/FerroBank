# FerroBank — Presentation & Demo Script (15-minute recording, team of 5)

A "sellable product" runsheet: a crisp pitch, a confident live demo, and each member
owning a segment (their group contribution **and** their individual extended feature).
Pairs with **[DEMO_SCENARIOS.md](./DEMO_SCENARIOS.md)** for the exact click-throughs.

> Per the spec's AI-use policy this is a runsheet to rehearse from, not a script to read
> verbatim — say it in your own words so the Q&A lands.

---

## The one-liner (positioning)

> **FerroBank is an iron-clad core-banking platform: it moves money safely under heavy
> concurrent load, proves every action with an audit trail, and enforces real-world
> bank controls — maker-checker approvals, OTP verification, and fraud monitoring.**

Three pillars to repeat throughout: **Safe. Accountable. Controlled.**

---

## Roles on the day

| Member | Segment | Group contribution | Individual extended feature (example) |
|---|---|---|---|
| **M1 — Platform Lead** | Intro + architecture + admin | Layered architecture, error handling, RBAC wiring | Admin dashboard + **fraud detection** (rejected / large / structuring / velocity) |
| **M2 — Auth** | Security | Registration, login, sessions, roles | **argon2 password hashing + role-based access control** |
| **M3 — Accounts** | Accounts | Accounts CRUD, balances | **Teller-approval workflow** (pending → active) + DB trigger |
| **M4 — Transfers** | The engine (headline) | Money movement, history | **Concurrency-safe transfer engine** (Mutex + row locks + rollback) + OTP |
| **M5 — Loans** | Lending | Loan lifecycle, repayments | **Dual-control loan approval** + repayment that debits an account |

---

## Slide deck (≈12 slides, ~5 min of talking + ~9 min demo)

| # | Slide | Speaker | ~Time | Key line / notes |
|---|---|---|---|---|
| 1 | Title — FerroBank, group #, names + IDs | M1 | 0:20 | "Iron-clad core banking, built in Rust." |
| 2 | The problem | M1 | 0:30 | Banks lose trust when money is lost, duplicated, or moved without a trail — especially under concurrent load. |
| 3 | Our solution + value pillars | M1 | 0:30 | Safe · Accountable · Controlled. One platform, role-aware. |
| 4 | Literature → decisions | M1 | 0:30 | Borrowed maker-checker, audit trails, OTP from Cyclos/Mambu/Fineract → justify dual approval + audit. |
| 5 | Architecture + OOP | M1 | 0:45 | Layered (handler→service trait→model→PG). Show the UML. Encapsulation, traits, polymorphism, Arc/Mutex. |
| 6 | Security (auth + RBAC) | M2 | 0:30 | argon2 hashing, encrypted sessions, 3 roles, `RequireRole` guard. |
| 7 | Accounts + approval | M3 | 0:20 | Customer opens → teller approves → active. DB enforces customers-only. |
| 8 | The transfer engine | M4 | 0:45 | Two-step + OTP; Mutex rate-limit + `FOR UPDATE` row locks + explicit rollback. This is the headline. |
| 9 | Fraud + audit | M1 | 0:20 | Rejected / large / structuring / velocity; every action audited + searchable. |
| 10 | Loans (dual control) | M5 | 0:20 | One teller + one admin must both approve; repayment debits a real account. |
| 11 | **LIVE DEMO** | all | ~9:00 | See runsheet below. |
| 12 | Wrap: what's sellable + roadmap | M1 | 0:30 | Trust by design; roadmap: statements/PDF, distributed limits, account-subtype traits. |

---

## Live demo runsheet (~9 min) — ordered for narrative impact

Reset to clean demo data first: `docker compose down -v && docker compose up --build`.
Have three browser sessions ready (customer, teller, admin). Numbers below reference
**DEMO_SCENARIOS.md**.

1. **(M2) RBAC tour — 1:00.** Logged-out landing → sign in as **alice** (customer view) →
   **teller** (lands on Loans, has `/staff` tabs, blocked from `/admin`) → **admin** (dashboard).
   *Say:* "Same app, three role-aware experiences — access enforced at the route layer." (Scenario 1)

2. **(M4) Transfer + OTP + audit — 2:00.** As alice, Transfers → New, pick from-account, paste
   bob's number, $250, submit. **Confirm page shows the recipient's name** + the OTP. Confirm →
   completed. Show bob's balance rose; show the `transfer.created`/`transfer.completed` audit rows.
   *Say:* "Two-step with OTP; the money only moves inside one locked transaction." (Scenario 2)

3. **(M4) Double-spend proof — 1:30.** Two tabs, two transfers from the *same* account that
   together exceed the balance; submit both confirmations together. **Exactly one succeeds**; the
   other is rejected for insufficient funds, with the reason shown. *Say:* "Row-level `FOR UPDATE`
   locks serialize them — no double spend, no negative balance." (Scenario 3) **← the money slide.**

4. **(M3) Account approval — 1:00.** As alice, open a new account → **Pending approval**, can't
   transact. As **teller** → `/staff/accounts` → **Approve** → Active. *Say:* "Maker-checker on
   account opening; the database itself blocks opening accounts for staff." (Scenario 10)

5. **(M5) Loan dual approval + repayment — 2:00.** As charlie, show the pending loan. **Teller**
   records approval (stays pending — "teller ✓ / admin pending"). **Admin** approves → **Approved**.
   Back as charlie, repay from a chosen account → balance drops, outstanding drops, 2-dp. *Say:*
   "Two different roles must sign off; repayment moves real money." (Scenario 8)

6. **(M1) Admin: fraud + CRUD + audit — 1:30.** Admin dashboard → **Flagged transfers** showing
   large/structuring/velocity reasons. Demo a $9,500 (structuring) flag. Freeze/adjust an account;
   search the audit log. *Say:* "Compliance surface: explainable fraud signals, full CRUD, every
   action logged and searchable." (Scenarios 6, 7, 9)

---

## 15-minute budget

| Block | Time |
|---|---|
| Pitch + architecture (slides 1–10) | ~5:00 |
| Live demo (6 steps) | ~9:00 |
| Wrap + roadmap | ~1:00 |

If running long, the two non-negotiables are **step 2 (OTP transfer)** and **step 3 (double-spend
proof)** — they are the spec's headline requirement.

---

## "Why it's a sellable product" talking points

- **Trust by design** — money can't be lost, duplicated, or sent without an audit record.
- **Real bank controls** — OTP, maker-checker dual approval, role separation, account freezing.
- **Explainable compliance** — fraud flags state *why*; the audit log is searchable.
- **Operationally honest** — graceful, explained rejections; no dead-ends for the user.
- **Engineered for scale-readiness** — async Actix workers, a connection pool, and a two-layer
  concurrency model (in-memory throttle + database row locks).

---

## Q&A prep (the 10% "individual understanding")

Each member should be ready for their area:

- **Concurrency (M4):** *"How do you prevent double spend?"* → one transaction, `SELECT … FOR UPDATE`
  on both accounts in id order (deadlock-safe), balance re-checked under the lock, explicit
  `rollback()` on failure. The Mutex is a separate, in-memory rate-limit layer.
- **Security (M2):** argon2id hashing (never plaintext), cookie sessions, `RequireRole` guard;
  admin is a superuser in the guard so `/staff` admits tellers and admins.
- **Accounts (M3):** pending→active lifecycle; a DB trigger guarantees only customers own accounts.
- **Loans (M5):** dual approval tracked in `loan_approvals` (UNIQUE per role ⇒ two distinct people);
  repayment + debit in one transaction.
- **Architecture/OOP (M1):** traits = abstraction, `dyn` = polymorphism, private fields =
  encapsulation, supertraits/composition = inheritance substitute; map to the four core objects.
- Likely curveball: *"What are the limits?"* → in-process rate limiter (single instance),
  simple-interest model, on-screen OTP for the demo. Naming these earns credibility.
