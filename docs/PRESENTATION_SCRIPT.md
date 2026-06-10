# FerroBank — Presentation Runsheet (15-minute recording, team of 5)

A runsheet to rehearse from, not to read verbatim. It mirrors
**[DEMO_SCENARIOS.md](./DEMO_SCENARIOS.md)** scene for scene — that file has the
exact click-throughs; this one has the words and the timing. The report skeleton
lives in **[REPORT_OUTLINE.md](./REPORT_OUTLINE.md)** and the original planning
draft in **[PROPOSAL.md](./PROPOSAL.md)**.

## The one-liner

> **FerroBank is an iron-clad core-banking platform: it moves money safely under
> heavy concurrent load, proves every action in an audit trail, and enforces
> real bank controls — maker-checker approvals, out-of-band OTPs, fraud holds
> with identity review, and automatic hijack defense.**

Three pillars to repeat: **Safe. Accountable. Controlled.**

## Segments & timing (≈15:00)

| # | Time | Scene (DEMO_SCENARIOS ref) | Owner |
|---|---|---|---|
| 1 | 0:00–1:30 | Architecture: FLOWS.md diagram 0, layered traits, SSR, ER diagram | M1 |
| 2 | 1:30–3:00 | Onboarding: register (names + NRIC) → sign in again → forced Telegram link, auto-refreshing guide (S1, S13) | M2 |
| 3 | 3:00–4:00 | Account opening with OTP → teller approval (S10) | M3 |
| 4 | 4:00–6:00 | Transfer with Telegram OTP → notifications toast on the recipient (S2, S11) | M4 |
| 5 | 6:00–8:00 | Race demo page + `cargo test` proof — locks, conservation, double-spend (S3) | M4 |
| 6 | 8:00–9:30 | Fraud hold → purpose + NRIC review → staff release/deny → hijack freeze (S15, S16) | M1 |
| 7 | 9:30–10:30 | Transfer limit consent popup + Alice's 10-second hold window (S14) | M3 |
| 8 | 10:30–12:00 | Loans: apply (OTP, disbursement account) → dual approval → principal lands → repayment (S8) | M5 |
| 9 | 12:00–13:00 | Admin: dashboard, audit log (show-more + per-user filter), inactivity logout (S6, S9, S17) | M1 |
| 10 | 13:00–13:30 | Ctrl-C → shutdown snapshot in psql (S12) | M2 |
| 11 | 13:30–15:00 | Each member: 15–20 s on their individual extended feature | all |

## Talking points per pillar

**Safe** — `SELECT … FOR UPDATE` row locks in ascending id order, explicit
rollback, Mutex rate limiter, money as `NUMERIC`/`rust_decimal`, the race-demo
invariant card ("money conserved ✓"), and the integration tests that assert no
overdraft / no loss / no double spend.

**Accountable** — append-only audit log with per-user action filter, every
request traced with user id + method + path, shutdown snapshots, notifications
on every decision, NRIC identity verification on fraud reviews.

**Controlled** — RBAC middleware (admin ⊃ teller ⊃ customer surfaces),
maker-checker on accounts, four-eyes on loans, OTP on every sensitive action
delivered out-of-band via Telegram, transfer limits with held increases,
5-minute inactivity TTL on database time, automatic freeze on suspected
hijack.

## Q&A bank (likely questions)

- *Why a Mutex AND row locks?* Different layers: the Mutex is cheap abuse
  protection before the DB; the locks are the correctness guarantee inside it.
- *Why not reentrant locking for the rate limit?* Reentrancy addresses a thread
  re-acquiring its own lock — never happens here; the 5/min cap is policy.
- *What happens on a crash?* Committed transactions are durable (Postgres WAL);
  the snapshot is a forensic marker on graceful shutdown, not recovery.
- *Why is the limit increase delayed?* A hijacked session must not be able to
  raise the ceiling and drain in one sitting; the old limit applies for 12 h.
- *Why Telegram instead of SMS?* Same out-of-band property, zero cost, real
  deliverability in a demo; the `OtpChannel` trait makes SMS a drop-in impl.
