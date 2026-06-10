# FerroBank — Demonstration Scenarios

A ready-to-run script for the project demonstration/recording. The scenarios are
chosen to map directly onto the **Banking System** domain in the project spec,
whose key focus is a *concurrency-safe money-transfer engine* plus audit logging,
OTP simulation, fraud rules, role-based access, and loan management.

Everything below uses the demo data created by the seed (`docker compose up --build`
runs it automatically). Account numbers are randomly generated, so where a step
needs one, read it off the relevant **Accounts** page rather than copying a fixed value.

---

## Seeded starting state

**Users** (all passwords follow the `<name>123` pattern):

| Email | Password | Role |
|---|---|---|
| `admin@ferrobank.local` | `admin123` | Admin |
| `teller@ferrobank.local` | `teller123` | Teller |
| `alice@ferrobank.local` | `alice123` | Customer |
| `bob@ferrobank.local` | `bob123` | Customer |
| `charlie@ferrobank.local` | `charlie123` | Customer |
| `diana@ferrobank.local` | `diana123` | Customer |
| `eve@ferrobank.local` | `eve123` | Customer |
| `frank@ferrobank.local` | `frank123` | Customer |

**Accounts / balances:**

| Owner | Type | Balance |
|---|---|---|
| Alice | Savings | $5,000.00 |
| Alice | Checking | $2,500.00 |
| Bob | Savings | $1,200.00 |
| Bob | Checking | $800.00 |
| Charlie | Checking | $350.00 |
| Diana | Savings | $0.00 |
| Eve | Savings | $7,500.00 |
| Eve | Checking | $1,500.00 |
| Frank | Checking | $600.00 |

**Loans:** Alice — $10,000 @ 5.25% / 36 mo (pending); Bob — $5,000 @ 7.00% / 24 mo
(approved); Charlie — $2,500 @ 6.50% / 12 mo (pending). Plus seeded historical
transfers and audit entries so the dashboards aren't empty — including one row
per fraud rule: a $9,999 "invoice" (structuring), a $12,500 transfer (large),
a rejected transfer, and a 4-transfer burst from Charlie (velocity).

> Seed data only inserts on first run against an empty table — it never resets
> balances or duplicates rows. To get the full fresh dataset (incl. Eve/Frank's
> fraud-demo transfers), recreate the database first.

---

## Scenario 1 — Role-based access control (RBAC)

**Goal:** show that the same app renders different capabilities per role.

1. Visit `http://localhost:8080` while logged out — landing page shows *Open an account* / *Sign in*.
2. Register a fresh customer live — the form takes **First / Middle (optional) / Last name** — and land on the home page greeted with **"Hello, {first name}"** over the animated hero background. Then sign in as **alice** (customer): nav shows **Accounts, Transfers, Loans**.
3. Sign out, sign in as **teller**. The teller lands on **Loans** (all loans, not just pending) and has their own **Accounts** and **Transfers** tabs under `/staff/*` — but is blocked from `/admin/*` (try `/admin/dashboard` → 403).
4. Sign out, sign in as **admin**. The nav now shows **Admin** + **Loans** (the customer Accounts/Transfers tabs are intentionally hidden in admin mode); admin manages those from the dashboard instead.
5. While signed in as alice, manually visit `/admin/dashboard` → you're redirected/blocked (the `RequireRole(Admin)` guard).

**Point out:** session-cookie auth (argon2-hashed passwords), the `CurrentUser` extractor, and the `RequireRole` middleware enforcing access at the route layer.

---

## Scenario 2 — Concurrency-safe transfer with OTP (headline feature)

**Goal:** demonstrate the spec's core requirement — an ACID money move with OTP confirmation and an audit trail.

1. Sign in as **alice**. Open **Accounts** and note her Checking account number and balance.
2. Open **bob**'s account number in advance (sign in as bob in another browser/profile, or just note it from a prior step) — you transfer *to* an account number.
3. As alice, go to **Transfers → New transfer**: choose the **from** account (dropdown of her active accounts), paste bob's account number as the recipient, enter an amount (e.g. `$250`), add a note, submit.
4. The **confirm page** shows the resolved **recipient name** (so you can verify who you're paying) and a demo **one-time code** (in production this would be an SMS).
5. Enter the OTP and confirm. Money moves; you're returned to history showing the completed transfer.
6. Sign in as **bob** → his balance increased by the same amount. Sign in as **admin → Audit log** → `transfer.created` and `transfer.completed` events are recorded.

**Point out:** the two-step create/confirm flow, the OTP hash stored (never plaintext), and that the actual balance change happens inside a single SQL transaction with `SELECT … FOR UPDATE` row locks on both accounts.

---

## Scenario 3 — Race conditions & double-spend prevention (the concurrency proof)

**Goal:** prove no money is lost or duplicated under simultaneous transactions.

**Method A — the Race Demo page (best on camera):**

1. Sign in as **admin → Dashboard → Race demo** (`/admin/race-demo`).
2. Pick a source account, set amount × tasks **greater than its balance** (e.g. $10 × 5 against a $30 balance — use Admin → Accounts → Adjust to set up a small balance first).
3. Fire. The results page shows every task's outcome and timing, the before/after balances, and a green **"Money conserved ✓"** card — exactly N transfers fit, the rest rejected with reasons, balance never negative.
4. Set tasks to 8+ to also show the **Mutex rate-limiter** rejecting the overflow before it reaches the database.

**Method B — manual two-tab version (shows it works for real users too):**

1. As **alice**, start **two** transfers from one account that *together* exceed its balance (e.g. 2 × $2,000 from $2,500).
2. Get both to the OTP confirm page in two tabs, submit both as close together as possible.
3. Exactly **one** completes; the other is **rejected: insufficient funds**.

**Method C — automated proof:** run `cargo test --test transfer_concurrency -- --nocapture` on screen — 5 simultaneous transfers and a double-confirm replay, all assertions green.

**Point out:** the row-level lock doing its job — the second transaction blocks on the first's `FOR UPDATE` lock, re-checks the balance under the lock, and rolls back. This is the spec's "prevent race conditions, inconsistent balances, and double spending."

---

## Scenario 4 — Rejection reasons & insufficient funds

**Goal:** show graceful, explained business-rule rejections.

1. As **alice**, attempt a transfer larger than the chosen account's balance and confirm it.
2. It's rejected, and the **reason is shown** ("insufficient funds: balance $X is less than $Y") in her transfer history and in the admin transfers view.
3. (Optional) As **admin**, freeze one of bob's accounts, then have bob attempt a transfer from it → rejected with "source account is frozen".

**Point out:** rejection reasons are persisted on the transfer row and surfaced in the UI and audit log. Also try transferring **between two of alice's own accounts** — refused immediately ("you cannot transfer to yourself"), enforced in the service AND by a database trigger.

---

## Scenario 5 — Application-level rate limiting (Mutex)

**Goal:** demonstrate the in-memory concurrency control layered on top of the DB locks.

1. As **alice**, make small valid transfers (e.g. $1) from the *same* account repeatedly.
2. On the 6th attempt within a minute, the create step is blocked with a **rate-limit** message.

**Point out:** this is a `tokio::sync::Mutex` over an in-memory per-account window — cheap protection that stops abusive bursts before they ever reach the database (complements, not replaces, the row locks).

---

## Scenario 6 — Fraud flagging

**Goal:** show the simple fraud-detection rule and the admin review surface.

1. Sign in as **admin → Dashboard**. The seed already trips **all four rules**, each with its reason shown in the **Flagged transfers** panel: the $12,500 Eve→Frank transfer (**large ≥ $10,000**), the $9,999 "invoice 4471" (**structuring**, just under the threshold), the **rejected** insufficient-funds transfer, and Charlie's 4-in-24h burst (**velocity**).
2. For a live one, send a $9,500 transfer from **eve**'s savings ($7,500 won't cover $10k, so structuring is the realistic live demo) and refresh the dashboard — it appears flagged immediately.
3. Open **Admin → Transfers** for the full searchable list.

**Point out:** fraud signals (rejected + high-value) are aggregated for staff, satisfying the spec's "fraud detection rules" advanced feature.

---

## Scenario 7 — Admin account management (CRUD) + audit

**Goal:** show full administrative control over user accounts with an audit trail.

1. Sign in as **admin → Accounts**. You see **every** account with owner, balance, and status, plus a search box.
2. **Open an account for a user:** pick Diana, choose *Checking*, submit → a new account appears.
3. **Adjust balance:** credit Diana's new account by `+500` (or debit with `-50`). The balance updates; a negative result is refused.
4. **Freeze / Unfreeze:** freeze one of Charlie's accounts, then unfreeze it.
5. **Close:** close a zero-balance account.
6. Open **Admin → Audit log** → every action above is recorded (`admin.account.opened`, `admin.account.adjusted`, `admin.account.frozen`, …).

**Point out:** complete CRUD across interconnected modules, each action audited — the spec's "complete CRUD functionality" and "audit logging".

---

## Scenario 8 — Loan dual approval + repayment that moves money

**Goal:** demonstrate a multi-role workflow and that a repayment actually debits an account.

1. Sign in as **charlie**, open **Loans**, and note the pending $2,500 application (or apply for a new one — the application is **OTP-confirmed** before it's submitted).
2. Sign in as **teller**, open that loan, and **record an approval**. The status stays *pending* — it now shows *Teller approved ✓ / Admin approval pending*.
3. Sign in as **admin**, open the same loan, and record the admin approval. Now it flips to **Approved** (it required two different roles).
4. Sign back in as **charlie**, open the approved loan, and **make a repayment**: choose which account to pay *from* (dropdown), enter an amount, submit.
5. Open **Accounts** → the funding account's balance has **decreased** by the repayment; the loan's outstanding balance drops, all shown to **2 decimal places**.

**Point out:** the dual-approval ledger (one teller + one admin, enforced distinct), and that repayment + debit happen in one transaction with a balance/active-account check.

---

## Scenario 9 — Audit log search

**Goal:** quick win showing the searchable compliance log.

1. As **admin → Audit log**, type `transfer` in the search box → only transfer events remain.
2. Type a user id or an event like `admin.account` → list narrows instantly (client-side filter over the latest 500 events).

---

## Scenario 10 — Account opening approval (teller workflow)

**Goal:** show the teller-approval control on new accounts.

1. Sign in as a **customer** (e.g. alice), go to **Accounts → Open new account**, choose a type, submit. A **one-time code page** appears first (opening an account is OTP-gated, like every sensitive action); enter the code. The new account then shows status **Pending approval** with a banner saying it's awaiting staff — it can't transact yet (no "Move money" button).
2. Sign out, sign in as **teller** → **Accounts** (`/staff/accounts`). The pending account shows an **Approve** button. Click it → status becomes **Active**; an `account.approved` audit entry is written.
3. Back as the customer, the account is now Active and can send money.

**Point out:** the `pending → active` lifecycle, the `RequireRole(Teller)` staff scope (admins pass it too), and that the database trigger blocks opening an account for a non-customer.

---

## Scenario 11 — Telegram OTP delivery (out-of-band verification)

**Goal:** show one-time codes arriving on a real phone instead of on screen.

1. Sign in as **alice** → **Settings** (nav) → *One-time codes in Telegram*. The page shows a 4-step guide and an **Open Telegram & link my account** button.
2. Click it — Telegram opens the FerroBank bot with a single-use code pre-filled. Press **Start**; the bot replies "✅ Linked!".
3. Refresh the settings page → shows **Telegram linked ✓**.
4. Make any transfer. The confirm page now says **"Check Telegram"** instead of showing the code; the 6-digit code arrives in the Telegram chat (show the phone on camera).
5. Enter it and complete the transfer. To unlink: either send **/unlink** to the bot (instant, updates the database from the chat) or use the website's unlink button — which is itself OTP-gated, since profile changes are sensitive actions. The bot also answers **/help** with its command list.

**Point out:** the `OtpChannel` trait with `TelegramOtp` / `ScreenOtp` impls — the transfer engine doesn't know which channel it's using (runtime polymorphism), and the OTP is still argon2-hashed at rest either way.

---

## Scenario 12 — Graceful-shutdown snapshot (terminal + database)

**Goal:** show the server's final act — a forensic state snapshot in the audit log.

1. With the server running in a terminal, press **Ctrl-C**. The log shows
   `server stopped — writing shutdown snapshot to audit_log` then `system.snapshot written`.
2. In psql:
   ```sql
   SELECT id, created_at, jsonb_pretty(payload) FROM audit_log
   WHERE event = 'system.snapshot' ORDER BY id DESC LIMIT 1;
   ```
   The payload records account/transfer/loan/repayment counts and total deposits at the moment of shutdown.
3. Restart the server — everything is intact (committed data was always durable via Postgres WAL; the snapshot is the ops/forensics marker).

---

## Suggested 15-minute recording running order

1. Architecture & stack overview (1–2 min) — `docs/FLOWS.md` diagram 0, layered design, traits, SSR.
2. Scenario 1 RBAC (1 min) — register a fresh user live, then tour the three roles.
3. Scenario 10 account approval (1 min) — flows naturally from the fresh registration: open account → pending → teller approves.
4. Scenario 2 transfer + OTP + audit (2 min) — the headline workflow.
5. Scenario 11 Telegram OTP (1–2 min) — same flow, code arrives on a phone.
6. Scenario 3 race demo page (2 min) — the technical highlight; add Method C's `cargo test` for proof.
7. Scenarios 4–5 rejection reasons + rate limit (1 min) — quick hits.
8. Scenario 8 loan dual approval + repayment (2 min).
9. Scenarios 6–7–9 fraud panel, admin CRUD, audit search (2 min).
10. Scenario 12 shutdown snapshot (30 s) — Ctrl-C, one psql query, done.
11. Each member explains their individual extended feature against the relevant scenario.

> Why this order: it follows one continuous story — a new customer onboards,
> gets approved, moves money, then the lens widens to engineering guarantees
> (concurrency), then staff/compliance surfaces, and ends on ops. Each scene
> sets up data the next scene uses, so there's no dead time on camera.

## How scenarios map to the marking criteria

| Spec focus | Scenario(s) |
|---|---|
| Concurrency-safe money transfer engine | 2, 3, 5 |
| Transaction audit logging | 2, 4, 7, 9, 12 |
| OTP simulation / secure verification | 2, 11 |
| Fraud detection rules | 6 |
| Role-based access control | 1, 7, 8 |
| CRUD across interconnected modules | 7, 8 |
| Business workflows (loans, approvals) | 8, 10 |
| SSR frontend & reusable components | all (shared layout, partials, themes) |
| Concurrency handling / real-time-ish updates (individual) | 3 (race demo page), 5 |
| Advanced auth / security workflows (individual) | 11 (Telegram), 1 |
