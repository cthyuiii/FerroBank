# FerroBank - Presentation Runsheet (15-minute recording, team of 5)

A runsheet to rehearse from, not to read verbatim. It mirrors
**[DEMO_SCENARIOS.md](./DEMO_SCENARIOS.md)** scene for scene - that file has the
exact click-throughs; this one has the words and the timing. The report skeleton
lives in **[REPORT_OUTLINE.md](./REPORT_OUTLINE.md)** and the original planning
draft in **[PROPOSAL.md](./PROPOSAL.md)**.

## The one-liner

> **FerroBank is an iron-clad core-banking platform: it moves money safely under
> heavy concurrent load, proves every action in an audit trail, and enforces
> real bank controls - maker-checker approvals, out-of-band OTPs, fraud holds
> with identity review, and automatic hijack defense.**

Three pillars to repeat: **Safe. Accountable. Controlled.**

## Segments & timing (≈15:00) - one continuous block per member

Each member presents exactly once and never returns. Every block opens with
that member's **flow chart from FLOWS.md** (15-20 seconds of "here is the
logic"), then plays the live scenarios that prove it. All 19 demo scenarios
are covered. Blocks chain their state: the customer M2 registers is the
account M3 approves; the money M4 moves funds M5's repayment.

| # | Time | Member block: flows first, then live scenarios |
|---|---|---|
| 1 | 0:00-2:45 | **M1 - Platform & Admin.** Flows **0** (stack) + **10** (sessions/TTL). Then live: RBAC tour + friendly 403 (S1), inactivity logout (S17), fraud panel tour (S6), audit show-more + per-user filter (S9). |
| 2 | 2:45-5:30 | **M2 - Auth & Identity.** Flows **1** (registration/login) + **9** (profile/linking) + **13** (device tracking). Then live: register with names + NRIC → sign in again → forced linking with the auto-refreshing page (S13), Telegram OTP delivery + bot /unlink and /help (S11), OTP-gated email/password change (S18), second-browser login → step-up code before any session exists (S20), new-device alert + the /staff/users view (S19), bot /unlink now scheduling 24h out with a cancellable red banner (S20). |
| 3 | 5:30-8:00 | **M3 - Accounts & Limits.** Flows **2** (opening/approval) + **6** (limit hold) + **8** (dual-control adjustments). Then live: open with OTP → pending page refreshes itself on teller approval (S10), Alice's 10-second limit hold maturing on camera (S14), admin CRUD + a parked >$1,000 adjustment approved by the teller (S7). |
| 4 | 8:00-12:00 | **M4 - The Transfer Engine.** Flows **3** (engine) + **4** (concurrency) + **5** (fraud hold) + **11** (notifications). Then live: transfer with Telegram OTP + named messages both ways (S2, S11 recap in passing), rejection reasons (S4), rate limit on the 6th attempt (S5), race demo page + `cargo test` (S3), fraud hold → purpose + NRIC → release/deny (S15), hijack freeze after 3 overdrafts (S16). |
| 5 | 12:00-14:40 | **M5 - Loans & Lifecycle.** Flows **7** (loan lifecycle) + **12** (shutdown snapshot). Then live: apply with OTP and a payout account → dual approval lands principal + due date → repayment advances it → paid off (S8); Ctrl-C → `system.snapshot` in psql closes the recording (S12). |
| 6 | 14:40-15:00 | **M1 voice-over outro** (pre-recorded is fine): the three pillars, one sentence each. |

Scenario coverage check: S1-S20 all appear exactly once above (S11 is owned by
M2; M4 only points at the toast arriving during the transfer).

## Talking points per pillar

**Safe** - `SELECT … FOR UPDATE` row locks in ascending id order, explicit
rollback, Mutex rate limiter, money as `NUMERIC`/`rust_decimal`, the race-demo
invariant card ("money conserved ✓"), and the integration tests that assert no
overdraft / no loss / no double spend.

**Accountable** - append-only audit log with per-user action filter, every
request traced with user id + method + path, shutdown snapshots, notifications
on every decision, NRIC identity verification on fraud reviews.

**Controlled** - RBAC middleware (admin ⊃ teller ⊃ customer surfaces),
maker-checker on accounts, four-eyes on loans, OTP on every sensitive action
delivered out-of-band via Telegram, transfer limits with held increases,
5-minute inactivity TTL on database time, automatic freeze on suspected
hijack.

## Q&A bank (likely questions)

- *Why a Mutex AND row locks?* Different layers: the Mutex is cheap abuse
  protection before the DB; the locks are the correctness guarantee inside it.
- *Why not reentrant locking for the rate limit?* Reentrancy addresses a thread
  re-acquiring its own lock - never happens here; the 5/min cap is policy.
- *What happens on a crash?* Committed transactions are durable (Postgres WAL);
  the snapshot is a forensic marker on graceful shutdown, not recovery.
- *Why is the limit increase delayed?* A hijacked session must not be able to
  raise the ceiling and drain in one sitting; the old limit applies for 12 h.
- *Why Telegram instead of SMS?* Same out-of-band property, zero cost, real
  deliverability in a demo; the `OtpChannel` trait makes SMS a drop-in impl.
