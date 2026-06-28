# FerroBank - Demonstration Scenarios

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

**Loans:** Alice - $10,000 @ 5.25% / 36 mo (pending); Bob - $5,000 @ 7.00% / 24 mo
(approved); Charlie - $2,500 @ 6.50% / 12 mo (pending). Plus seeded historical
transfers and audit entries so the dashboards aren't empty - including a
$12,500 transfer (large), a flagged rejection, and a 4-transfer burst from
Charlie (velocity). Structuring is balance-relative now, so it's demoed live
(scenario 6) rather than seeded.

> Seed data only inserts on first run against an empty table - it never resets
> balances or duplicates rows. To get the full fresh dataset (incl. Eve/Frank's
> fraud-demo transfers), recreate the database first.

---

## Scenario 1 - Role-based access control (RBAC)

**Goal:** show that the same app renders different capabilities per role.

1. Visit `http://localhost:8080` while logged out - landing page shows *Open an account* / *Sign in*.
2. Register a fresh customer live - the form takes **First / Middle (optional) / Last name** - and land on the home page greeted with **"Hello, {first name}"** over the animated hero background. Then sign in as **alice** (customer): nav shows **Accounts, Transfers, Loans**.
3. Sign out, sign in as **teller**. The teller lands on **Loans** (all loans, not just pending) and has their own **Accounts** and **Transfers** tabs under `/staff/*` - but is blocked from `/admin/*` (try `/admin/dashboard` → 403).
4. Sign out, sign in as **admin**. The nav now shows **Admin** + **Loans** (the customer Accounts/Transfers tabs are intentionally hidden in admin mode); admin manages those from the dashboard instead.
5. While signed in as alice, manually visit `/admin/dashboard` → you're redirected/blocked (the `RequireRole(Admin)` guard).

**Point out:** session-cookie auth (argon2-hashed passwords), the `CurrentUser` extractor, and the `RequireRole` middleware enforcing access at the route layer.

---

## Scenario 2 - Concurrency-safe transfer with OTP (headline feature)

**Goal:** demonstrate the spec's core requirement - an ACID money move with OTP confirmation and an audit trail.

1. Sign in as **alice**. Open **Accounts** and note her Checking account number and balance.
2. Open **bob**'s account number in advance (sign in as bob in another browser/profile, or just note it from a prior step) - you transfer *to* an account number.
3. As alice, go to **Transfers → New transfer**: choose the **from** account (dropdown of her active accounts), paste bob's account number as the recipient, enter an amount (e.g. `$250`), add a note, submit.
4. The **confirm page** shows the resolved **recipient name** (so you can verify who you're paying) and a demo **one-time code** (in production this would be an SMS).
5. Enter the OTP and confirm. Money moves; you're returned to history showing the completed transfer.
6. Sign in as **bob** → his balance increased by the same amount. Sign in as **admin → Audit log** → `transfer.created` and `transfer.completed` events are recorded.

**Point out:** the two-step create/confirm flow, the OTP hash stored (never plaintext), and that the actual balance change happens inside a single SQL transaction with `SELECT … FOR UPDATE` row locks on both accounts.

---

## Scenario 3 - Race conditions & double-spend prevention (the concurrency proof)

**Goal:** prove no money is lost or duplicated under simultaneous transactions.

**Method A - the Race Demo page (best on camera):**

1. Sign in as **admin → Dashboard → Race demo** (`/admin/race-demo`).
2. Pick a source account, set amount × tasks **greater than its balance** (e.g. $10 × 5 against a $30 balance - use Admin → Accounts → Adjust to set up a small balance first).
3. Fire. The results page shows every task's outcome and timing, the before/after balances, and a green **"Money conserved ✓"** card - exactly N transfers fit, the rest rejected with reasons, balance never negative.
4. Set tasks to 8+ to also show the **Mutex rate-limiter** rejecting the overflow before it reaches the database.

**Method B - manual two-tab version (shows it works for real users too):**

1. As **alice**, start **two** transfers from one account that *together* exceed its balance (e.g. 2 × $2,000 from $2,500).
2. Get both to the OTP confirm page in two tabs, submit both as close together as possible.
3. Exactly **one** completes; the other is **rejected: insufficient funds**.

**Method C - automated proof:** run `cargo test --test transfer_concurrency -- --nocapture` on screen - 5 simultaneous transfers and a double-confirm replay, all assertions green.

**Point out:** the row-level lock doing its job - the second transaction blocks on the first's `FOR UPDATE` lock, re-checks the balance under the lock, and rolls back. This is the spec's "prevent race conditions, inconsistent balances, and double spending."

---

## Scenario 4 - Rejection reasons & insufficient funds

**Goal:** show graceful, explained business-rule rejections.

1. As **alice**, attempt a transfer larger than the chosen account's balance and confirm it.
2. It's rejected, and the **reason is shown** ("insufficient funds: balance $X is less than $Y") in her transfer history and in the admin transfers view.
3. (Optional) As **admin**, freeze one of bob's accounts, then have bob attempt a transfer from it → rejected with "source account is frozen".

**Point out:** rejection reasons are persisted on the transfer row and surfaced in the UI and audit log. Also try transferring **between two of alice's own accounts** - refused immediately ("you cannot transfer to yourself"), enforced in the service AND by a database trigger.

---

## Scenario 5 - Application-level rate limiting (Mutex)

**Goal:** demonstrate the in-memory concurrency control layered on top of the DB locks.

1. As **alice**, make small valid transfers (e.g. $1) from the *same* account repeatedly.
2. On the 6th attempt within a minute, the create step is blocked with a **rate-limit** message.

**Point out:** this is a `tokio::sync::Mutex` over an in-memory per-account window - cheap protection that stops abusive bursts before they ever reach the database (complements, not replaces, the row locks).

---

## Scenario 6 - Fraud flagging

**Goal:** show the simple fraud-detection rule and the admin review surface.

1. Sign in as **admin → Dashboard**. The **Flagged transfers** panel runs the rules with each reason shown: **large** transfers (>= $10,000), **structuring** (a sub-limit transfer out of a balance >= $5,000 that empties the account to within $49), held transfers with their stored reason, non-funds **rejections**, and **velocity** (4+ transfers from one account within 1 hour). Insufficient funds is NOT flagged: it is pre-checked before anything posts.
2. For a live structuring demo: from **eve**'s savings ($7,500), send $7,460 (leaves under $49, stays under the limit). It lands on hold and appears flagged immediately.
3. Open **Admin → Transfers** for the full searchable list.

**Point out:** fraud signals (rejected + high-value) are aggregated for staff, satisfying the spec's "fraud detection rules" advanced feature.

---

## Scenario 7 - Admin account management (CRUD) + audit

**Goal:** show full administrative control over user accounts with an audit trail.

1. Sign in as **admin → Accounts**. You see **every** account with owner, balance, and status, plus a search box.
2. **Open an account for a user:** pick Diana, choose *Checking*, submit → a new account appears.
3. **Adjust balance:** credit Diana's new account by `+500` (or debit with `-50`). The balance updates; a negative result is refused.
4. **Freeze / Unfreeze:** freeze one of Charlie's accounts, then unfreeze it.
5. **Close:** close a zero-balance account.
6. Open **Admin → Audit log** → every action above is recorded (`admin.account.opened`, `admin.account.adjusted`, `admin.account.frozen`, …).

**Point out:** complete CRUD across interconnected modules, each action audited - the spec's "complete CRUD functionality" and "audit logging".

---

## Scenario 8 - Loan dual approval + repayment that moves money

**Goal:** demonstrate a multi-role workflow and that a repayment actually debits an account.

1. Sign in as **charlie**, open **Loans**, and note the pending $2,500 application (or apply for a new one - the application is **OTP-confirmed** before it's submitted).
2. Sign in as **teller**, open that loan, and **record an approval**. The status stays *pending* - it now shows *Teller approved ✓ / Admin approval pending*.
3. Sign in as **admin**, open the same loan, and record the admin approval. Now it flips to **Approved** (it required two different roles).
4. Sign back in as **charlie**, open the approved loan, and **make a repayment**: choose which account to pay *from* (dropdown), enter an amount, submit.
5. Open **Accounts** → the funding account's balance has **decreased** by the repayment; the loan's outstanding balance drops, all shown to **2 decimal places**.

**Point out:** the dual-approval ledger (one teller + one admin, enforced distinct), and that repayment + debit happen in one transaction with a balance/active-account check.

---

## Scenario 9 - Audit log search

**Goal:** quick win showing the searchable compliance log.

1. As **admin → Audit log**, type `transfer` in the search box → only transfer events remain.
2. Type a user id or an event like `admin.account` → list narrows instantly (client-side filter over the latest 500 events).

---

## Scenario 10 - Account opening approval (teller workflow)

**Goal:** show the teller-approval control on new accounts.

1. Sign in as a **customer** (e.g. alice), go to **Accounts → Open new account**, choose a type, submit. A **one-time code page** appears first (opening an account is OTP-gated, like every sensitive action); enter the code. The new account then shows status **Pending approval** with a banner saying it's awaiting staff - it can't transact yet (no "Move money" button).
2. Sign out, sign in as **teller** → **Accounts** (`/staff/accounts`). The pending account shows an **Approve** button. Click it → status becomes **Active**; an `account.approved` audit entry is written.
3. Back as the customer, the account is now Active and can send money.

**Point out:** the `pending → active` lifecycle, the `RequireRole(Teller)` staff scope (admins pass it too), and that the database trigger blocks opening an account for a non-customer.

---

## Scenario 11 - Telegram OTP delivery (out-of-band verification)

**Goal:** show one-time codes arriving on a real phone instead of on screen.

1. Sign in as **alice** → **Settings** (nav) → *One-time codes in Telegram*. The page shows a 4-step guide and an **Open Telegram & link my account** button.
2. Click it - Telegram opens the FerroBank bot with a single-use code pre-filled. Press **Start**; the bot replies "✅ Linked!".
3. Refresh the settings page → shows **Telegram linked ✓**.
4. Make any transfer. The confirm page now says **"Check Telegram"** instead of showing the code; the 6-digit code arrives in the Telegram chat (show the phone on camera).
5. Enter it and complete the transfer. To unlink: either send **/unlink** to the bot (instant, updates the database from the chat) or use the website's unlink button - which is itself OTP-gated, since profile changes are sensitive actions. The bot also answers **/help** with its command list.

**Point out:** the `OtpChannel` trait with `TelegramOtp` / `ScreenOtp` impls - the transfer engine doesn't know which channel it's using (runtime polymorphism), and the OTP is still argon2-hashed at rest either way.

---

## Scenario 12 - Graceful-shutdown snapshot (terminal + database)

**Goal:** show the server's final act - a forensic state snapshot in the audit log.

1. With the server running in a terminal, press **Ctrl-C**. The log shows
   `server stopped - writing shutdown snapshot to audit_log` then `system.snapshot written`.
2. In psql:
   ```sql
   SELECT id, created_at, jsonb_pretty(payload) FROM audit_log
   WHERE event = 'system.snapshot' ORDER BY id DESC LIMIT 1;
   ```
   The payload records account/transfer/loan/repayment counts and total deposits at the moment of shutdown.
3. Restart the server - everything is intact (committed data was always durable via Postgres WAL; the snapshot is the ops/forensics marker).

---

## Scenario 13 - Mandatory Telegram linking

**Goal:** show that a customer cannot use the bank at all until an out-of-band channel exists.

1. Register a fresh customer - the form asks **First / Middle (optional) / Last name and NRIC**.
2. The page confirms registration and sends you to sign in (no auto-login).
3. Sign in. You are routed straight to the **Telegram linking guide** - and every other page redirects back here until linked (only Settings, logout, and notifications are reachable).
4. Click **Open Telegram & link my account** - the bot opens with a single-use code pre-filled; press Start.
5. The linking page polls and refreshes itself the instant the bot confirms. You're in.
6. From now on, every one-time code arrives on the phone; nothing appears on screen.

**Point out:** the enforcement lives in `ActivityGuard` middleware, so no handler can forget it; the deep-link code is single-use; the page polls `/settings/telegram/status` rather than asking the user to refresh.

---

## Scenario 14 - Transfer limit with hold window

**Goal:** show user-controlled limits where increases cool off before applying.

1. Sign in as **alice**, open one of her accounts. The **Transfer limit** card shows the current per-transfer limit.
2. Request an *increase*. A consent popup warns that increases are held before they apply. Confirm, then enter the one-time code (limit changes are OTP-gated).
3. The page shows the pending change with the moment it takes effect **in your local time**. Normally that's 12 hours; alice is seeded to **10 seconds** so it can be waited out on camera.
4. Before maturity, try a transfer above the OLD limit - it is still refused (the old limit applies until the hold matures).
5. After 10 seconds, refresh and send the same transfer - it now passes.
6. Request a *decrease* - it applies immediately, no hold.

**Point out:** matured changes are promoted lazily on every account read and before every transfer (no background job needed), and the asymmetry - tightening is instant, loosening waits - is exactly how real banks treat risk-increasing changes.

---

## Scenario 15 - Fraud hold + identity review

**Goal:** show a suspicious transfer being parked, the customer proving identity, and staff deciding.

1. Sign in as **eve** (savings $7,500). Create a transfer that drains more than half the balance (e.g. $4,000), confirm with the OTP.
2. Instead of completing, the transfer lands **on hold** - no money has moved - and eve is told her transfer needs review, with a toast and a Telegram message.
3. The review page asks for the **purpose** of the transfer and her **NRIC**; submit both. The page polls status every few seconds.
4. Sign in as **teller** (or admin) → **Transfers** → the held-transfers queue. The claim shows the stated purpose and NRIC **beside the NRIC on file**, so identity is a visual match.
5. **Release** it - the locked re-checks run, money moves, both parties get named notifications. (Or **deny** with a reason - the sender is told.)

**Point out:** the four rules (>= $10k, structuring, >50% drain of a >$5k balance, 4+ transfers/hour) are evaluated under the same row locks as the money move, so a hold can never race a completion; insufficient funds is pre-checked at creation and never reaches review.

---

## Scenario 16 - Hijack freeze (3 overdraft attempts)

**Goal:** show the automatic defense against an attacker probing a compromised account.

1. Sign in as a customer with a small balance (e.g. **frank**, $600).
2. Attempt a transfer larger than the balance - it is rejected at the create step ("insufficient funds") and a rejected row is recorded. No OTP is ever issued.
3. Repeat twice more within 24 hours (different amounts are fine).
4. On the third rejection the account is **frozen automatically** and the owner gets a toast + Telegram alert explaining why.
5. Any further transfer attempt from that account fails with "source account is frozen". Staff unfreeze it from the accounts page once the owner is verified.

**Point out:** repeated overdraft attempts are a hijack signature (an attacker doesn't know the balance); the counter is per-account over a rolling 24 hours, and the freeze + alert are audited.

---

## Scenario 17 - Inactivity timeout

**Goal:** show the server-side session TTL.

1. Sign in as any user and leave the tab idle for 5 minutes.
2. Click anything - you are back at the login page with "signed out after 5 minutes of inactivity".
3. (For a faster recording, shrink the interval in `ActivityGuard` beforehand.)
4. Sign in again - the timer resets (login refreshes `last_activity_at`).

**Point out:** the TTL is enforced on **database time**, not in the browser - clearing cookies or freezing the client clock cannot bypass it - and every authenticated request is logged with user id, method, and path, which is what feeds the per-user activity trail.

---

## Scenario 18 - OTP discipline everywhere (profile changes + 3 strikes)

**Goal:** show that the same one-time-code guard protects every sensitive action, with a uniform wrong-code budget.

1. Sign in as a linked customer → **Settings**. Change the **email** - a code arrives in Telegram; enter it and the change applies. Repeat for **password**.
2. Now start any OTP-gated action (a transfer is easiest) and type a **wrong code**. The page re-renders inline with "invalid confirmation code (attempt 1 of 3)" - no progress lost.
3. Type two more wrong codes. On the third, the pending action is **cancelled outright** ("too many invalid codes").
4. Start again with the right code to show recovery is just re-initiating.

**Point out:** one shared `ActionOtpService` guards account opening, loan applications, limit changes, profile changes, unlinking, and risky logins; codes are argon2-hashed at rest, single-use, expire in 10 minutes, and the 3-strike budget is identical everywhere.

---

## Scenario 19 - Login device tracking

**Goal:** show per-login device intelligence and the staff investigation view.

1. Sign in as **alice** from your usual browser - nothing special happens (known origin).
2. Sign in as alice from a **different browser** (or a private window with a different user agent).
3. Alice immediately gets a toast + Telegram **security alert** naming the browser family and IP - "new device" / "new network" - with "change your password if this wasn't you".
4. Sign in as **teller or admin**, open **/staff/users/{alice's id}** (click her name on the staff accounts page).
5. Walk the page: identity card (email, NRIC, Telegram status, customer since), the **login history** with *New device* / *New network* badges per row, then her accounts and latest transfers - one screen for a takeover investigation.

**Point out:** every login inserts a `login_sessions` row (browser family, IP, first-seen flags); the very first login ever is exempt from alerts; pairing the login signals with held transfers is how staff confirm or clear a suspected takeover.

---

## Scenario 20 - Step-up login, hardened unlink, and origin blocking

**Goal:** show the layered defense against a stolen password and a stolen Telegram.

1. From the **second browser** (now a known-but-flagged origin from scenario 19), log out and log in as alice again from a fresh private window: after the correct password, a **"Verify it's you"** page demands a one-time code from her Telegram. **No session exists yet** - the cookie is only set after the code verifies. Known origins stay password-only.
2. Enter the code - you land signed in, and the step-up pass is audited.
3. Now the stolen-Telegram side: send **/unlink** to the bot. Instead of disconnecting, the bot replies that the unlink takes effect in **24 hours**, alice is alerted on every channel, and her **Settings** page shows a red banner with a **Cancel the unlink** button.
4. Click cancel - it requires her password-backed website session, which is exactly what a Telegram thief doesn't have. (The website's own OTP-confirmed unlink stays immediate - the owner proves control of both factors.)
5. Bonus beat: from yet another private window, reach the step-up page and type **3 wrong codes**. The attempt is cancelled and that browser + network is **blocked from alice's account for 24 hours**, even with the correct password. The block appears as a red **Active sign-in blocks** panel on her staff profile page.

**Point out:** the password alone is never enough from somewhere new; the unlink delay turns "instant de-factoring" into a 24-hour race the real owner wins; and the origin block stops an attacker from brute-forcing the step-up code.

---

## Recording running order - one pass per member, flow chart first

Each member appears exactly once. Every block opens with the member's flow
chart(s) from [FLOWS.md](./FLOWS.md), then the live scenarios. All 20
scenarios are used.

| Block | Member | Flow charts | Live scenarios | Hand-off |
|---|---|---|---|---|
| 1 | M1 Platform & Admin | 0, 10 | S1, S17, S6, S9 | admin stays signed in for later approvals |
| 2 | M2 Auth & Identity | 1, 9, 13 | S13, S11, S18, S19, S20 | the new customer exists, is linked, and has tripped a device alert and a step-up |
| 3 | M3 Accounts & Limits | 2, 6, 8 | S10, S14, S7 | funded active accounts with a raised limit |
| 4 | M4 Transfer Engine | 3, 4, 5, 11 | S2, S4, S5, S3, S15, S16 | money in bob's account funds the loan story |
| 5 | M5 Loans & Close | 7, 12 | S8, S12 | Ctrl-C ends the recording naturally |

> One continuous story: platform context → a new identity onboards and is
> tracked → accounts exist with limits → money moves and is policed → credit,
> then the closing snapshot. Nobody appears twice.

## How scenarios map to the marking criteria

| Spec focus | Scenario(s) |
|---|---|
| Concurrency-safe money transfer engine | 2, 3, 5 |
| Transaction audit logging | 2, 4, 7, 9, 12 |
| OTP simulation / secure verification | 2, 11, 13, 18, 20 |
| Fraud detection rules | 6, 15, 16 |
| Role-based access control | 1, 7, 8, 19 |
| CRUD across interconnected modules | 7, 8 |
| Business workflows (loans, approvals) | 8, 10, 14 |
| SSR frontend & reusable components | all (shared layout, partials, themes) |
| Concurrency handling / real-time-ish updates (individual) | 3 (race demo page), 5 |
| Advanced auth / security workflows (individual) | 11, 13, 17, 18, 19, 20 |


