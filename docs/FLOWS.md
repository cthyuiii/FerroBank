# FerroBank — System & Workflow Flow Diagrams

Sequence/flow diagrams for every demonstrable scenario: the implemented core
workflows first, then the **planned additional features** (marked *PLANNED*).
Render with any Mermaid viewer (GitHub, VS Code + Mermaid extension,
<https://mermaid.live>) — handy for the report, slides, and the demo recording.

Actors used throughout: **User** (person), **Browser**, **Actix** (middleware +
handler), **Service** (business logic), **PostgreSQL**, plus **Telegram** for
the OTP delivery channel.

---

## 0. Tech stack — how the systems talk to each other

```mermaid
flowchart LR
    subgraph Client
        U[User] --> B["Browser<br/>(SSR HTML + Tailwind CDN + HTMX)"]
    end

    subgraph Server["Actix Web (Rust)"]
        MW["Middleware<br/>session cookie · CurrentUser · RequireRole · tracing"]
        H["Handlers (thin)<br/>parse forms · pick templates"]
        SV["Service traits (thick)<br/>Auth · Account · Transfer · Loan · Audit · Admin<br/>Arc&lt;dyn Trait&gt; — OOP polymorphism"]
        T["Askama templates<br/>(compile-time checked)"]
    end

    subgraph Data["PostgreSQL 16"]
        DB[("users · accounts · transfers<br/>loans · loan_approvals · repayments<br/>action_otps · audit_log")]
    end

    EXT["Telegram Bot API<br/>(OTP delivery + /unlink commands)"]

    B -- "HTTP GET/POST (forms)" --> MW --> H --> SV
    SV -- "SQLx: transactions,<br/>SELECT … FOR UPDATE" --> DB
    SV -. "audit every state change" .-> DB
    H --> T -- "HTML response" --> B
    SV -. "HTTPS sendMessage" .-> EXT
    SEED["seed binary<br/>(cargo run --bin seed)"] -- "idempotent demo data" --> DB
    MAIN["main.rs"] -- "migrations on startup ·<br/>system.snapshot on shutdown" --> DB
```

Request path in one line: **Browser → session middleware → role guard →
handler → service trait → SQLx transaction → PostgreSQL → Askama template →
HTML back to the browser.** No JSON API, no client-side framework — every page
is server-side rendered; HTMX progressively enhances form posts.

---

## 1. Registration & login (sessions)

```mermaid
sequenceDiagram
    actor U as User
    participant B as Browser
    participant A as Actix (auth handler)
    participant S as AuthService
    participant P as PostgreSQL

    U->>B: fill /register form
    B->>A: POST /register (email, first/middle/last name, password)
    A->>A: validator: email format, names, password ≥ 8 chars
    A->>S: register(NewUser{role: Customer})
    S->>S: argon2id hash (unique salt)
    S->>P: INSERT INTO users … (UNIQUE email)
    P-->>S: user row
    S-->>A: User
    A->>A: session::login() → signed cookie (id, email, role)
    A-->>B: 302 → /accounts (role-based landing)<br/>home page greets "Hello, {first name}"

    Note over B,P: Login: same shape — verify argon2 hash,<br/>identical error for wrong email vs wrong password (no user enumeration)
```

## 2. Account opening with maker–checker approval

```mermaid
sequenceDiagram
    actor C as Customer
    actor T as Teller/Admin
    participant A as Actix
    participant S as AccountService
    participant P as PostgreSQL

    C->>A: POST /accounts/new (savings | checking)
    A->>A: OTP gate: park request in action_otps,<br/>send code (Telegram or on-screen)
    C->>A: POST /accounts/new/confirm (code)
    A->>A: verify: owner-bound · single-use · 10-min expiry
    A->>S: open_account(user, kind, approved=false)
    S->>P: INSERT account status='pending'<br/>(trigger: owner must be a customer)
    P-->>C: account visible, but PENDING — cannot transact

    T->>A: GET /staff/accounts (RequireRole(Teller))
    A-->>T: pending-accounts queue
    T->>A: POST /staff/accounts/{id}/approve
    A->>S: approve_account(id)
    S->>P: UPDATE … SET status='active' WHERE status='pending'
    A->>P: audit_log: account.approved (actor = teller)
    P-->>C: account ACTIVE — can now send/receive money
```

## 3. Money transfer — two-step with OTP (core engine)

```mermaid
sequenceDiagram
    actor U as Customer
    participant B as Browser
    participant A as Actix (transfers handler)
    participant S as TransferService
    participant P as PostgreSQL

    U->>B: transfer form (from, to acct number, amount)
    B->>A: POST /transfers/new
    A->>A: parse Decimal · sender must OWN from-account
    A->>S: create(actor, from, to, amount)
    S->>P: recipient must belong to a DIFFERENT user<br/>(no self-transfers; DB trigger backs this up)
    S->>S: Mutex rate-limit (≤5/min per account)
    S->>S: generate 6-digit OTP → argon2 hash
    S->>P: INSERT transfer status='pending' + otp_hash
    S->>P: audit_log: transfer.created
    S-->>B: confirm page (recipient name + demo OTP)

    U->>B: types OTP
    B->>A: POST /transfers/confirm
    A->>S: confirm(actor, transfer_id, otp)
    S->>P: BEGIN
    S->>P: SELECT transfer FOR UPDATE (must be 'pending')
    S->>P: actor must own the source account (else Forbidden)
    S->>S: verify OTP against argon2 hash
    S->>P: SELECT both accounts FOR UPDATE — ascending id (no deadlock)
    S->>S: re-check under lock: active? balance ≥ amount?
    alt all checks pass
        S->>P: debit · credit · status='completed' (one txn)
        S->>P: COMMIT
        S->>P: audit_log: transfer.completed
    else any check fails
        S->>P: status='rejected' + human-readable reason · COMMIT
        S->>P: audit_log: transfer.rejected
    end
    A-->>B: 302 → /transfers (history shows outcome + reason)
```

## 4. Concurrency: why simultaneous transfers can't corrupt balances

```mermaid
sequenceDiagram
    participant T1 as Transfer task 1 ($10)
    participant T2 as Transfer task 2 ($10)
    participant P as PostgreSQL (account balance: $15)

    par simultaneous confirms
        T1->>P: SELECT … FOR UPDATE  (acquires row lock)
        T2->>P: SELECT … FOR UPDATE  (BLOCKS, waits for T1)
    end
    T1->>P: balance 15 ≥ 10 ✓ → debit → COMMIT (balance: $5)
    Note over T2,P: lock released — T2 now reads the REAL balance
    T2->>P: balance 5 ≥ 10 ✗ → status='rejected' (insufficient funds)
    Note over T1,T2: Proven by tests/transfer_concurrency.rs:<br/>no overdraft, no lost money, no double spend
```

## 5. Loan lifecycle — dual approval (four-eyes) + repayment

```mermaid
sequenceDiagram
    actor C as Customer
    actor T as Teller
    actor AD as Admin
    participant S as LoanService
    participant P as PostgreSQL

    C->>S: apply(principal, rate, term)
    Note over C,S: OTP gate first: application parked in action_otps,<br/>submitted only after the code verifies
    S->>P: INSERT loan status='pending'

    T->>S: approve(loan, teller)
    S->>P: INSERT loan_approvals (loan, role='teller')<br/>UNIQUE(loan_id, role)
    Note over S,P: still pending — only 1 of 2 slots filled

    AD->>S: approve(loan, admin)
    S->>P: INSERT loan_approvals (loan, role='admin')
    S->>P: both slots filled → UPDATE loans SET status='approved'

    C->>S: record_repayment(loan, amount, funding account)
    S->>P: BEGIN · lock loan + funding account FOR UPDATE
    S->>S: account active? balance ≥ amount?
    S->>P: debit account · INSERT repayment · recompute outstanding
    alt outstanding ≤ 0
        S->>P: status='paid_off' · COMMIT
    else still owing
        S->>P: status='active' · COMMIT
    end
```

## 6. Graceful shutdown — system snapshot to the audit log

```mermaid
sequenceDiagram
    participant OS as OS signal (Ctrl-C / SIGTERM)
    participant M as main.rs
    participant AX as Actix HttpServer
    participant P as PostgreSQL

    OS->>AX: SIGINT / SIGTERM
    AX->>AX: graceful shutdown — finish in-flight requests
    AX-->>M: run() returns
    M->>P: SELECT counts + SUM(balances) across the bank
    M->>P: INSERT audit_log event='system.snapshot' (JSON payload)
    M-->>OS: exit 0
    Note over M,P: Hard crash (kill -9 / power loss): nothing can run —<br/>but committed transactions are already durable (Postgres WAL).<br/>The snapshot is a forensic marker, not a recovery mechanism.
```

---

# Planned additional features

## 7. OTP delivery via Telegram bot (IMPLEMENTED)

```mermaid
sequenceDiagram
    actor U as Customer
    participant B as Browser
    participant A as Actix
    participant S as TransferService
    participant TG as Telegram Bot API
    participant P as PostgreSQL

    Note over U,P: One-time linking
    U->>B: profile page → "Link Telegram" deep link<br/>https://t.me/FerroBankBot?start=&lt;one-time code&gt;
    U->>TG: taps Start (sends /start &lt;code&gt;)
    TG-->>A: getUpdates poll: /start &lt;code&gt; + chat_id
    A->>P: match code → save chat_id (+ phone) on user

    Note over U,P: Every transfer afterwards
    U->>A: POST /transfers/new
    A->>S: create(…) → OTP generated, argon2-hashed in DB
    S->>TG: POST /bot&lt;TOKEN&gt;/sendMessage {chat_id, "Your FerroBank code: 123456"}
    TG-->>U: OTP arrives in the user's Telegram
    U->>A: POST /transfers/confirm (types code from phone)
    A->>S: confirm(…) — unchanged from flow 3
    Note over S: Users without a linked chat_id fall back to<br/>the on-screen demo OTP (current behaviour)
```

## 8. PLANNED — High-value transfer consent (> $5,000)

```mermaid
sequenceDiagram
    actor U as Customer
    participant B as Browser
    participant A as Actix
    participant S as TransferService
    participant P as PostgreSQL

    U->>B: submit transfer of $7,500
    B->>A: POST /transfers/new
    A->>S: create(…)
    S->>S: amount > $5,000 → flag requires_consent
    S-->>B: confirm page + consent modal:<br/>"High-value transfer — I understand and consent"
    U->>B: ticks consent + enters OTP
    B->>A: POST /transfers/confirm (otp, consent=true)
    A->>S: confirm(…)
    S->>P: verify consent recorded (consented_at) — refuse without it
    S->>P: usual locked money move (flow 3)
    S->>P: audit_log: transfer.high_value_consented {amount, actor}
```

## 9. PLANNED — Per-account transfer limit with 12-hour hold

```mermaid
sequenceDiagram
    actor U as Customer
    participant A as Actix
    participant S as AccountService
    participant P as PostgreSQL

    U->>A: POST /accounts/{id}/limit (new limit $2,000 → $8,000)
    A->>A: consent modal (same flow as high-value transfers)
    A->>S: request_limit_change(account, new_limit)
    S->>P: INSERT limit_changes {account, old, new,<br/>effective_at = now() + 12h, status='pending'}
    S->>P: audit_log: account.limit_change_requested
    Note over U,P: For 12 hours the OLD limit still applies —<br/>transfers above it are rejected with a clear reason.<br/>(Anti-fraud: a hijacked session can't instantly raise limits and drain.)

    U->>A: any transfer after effective_at
    A->>S: create(…)
    S->>P: lazily apply matured limit change (status='applied')
    S->>S: enforce: amount ≤ account.transfer_limit
```

## 10. PLANNED — Flagged-transfer hold + staff intervention

```mermaid
sequenceDiagram
    actor U as Customer
    actor ST as Teller/Admin
    participant S as TransferService
    participant P as PostgreSQL

    U->>S: confirm(…) on a transfer matching a fraud rule<br/>(≥ $10k · $9k–10k structuring · 4+ in 24h velocity)
    S->>P: instead of completing: status='on_hold' + reason<br/>(money NOT moved; nothing debited yet)
    S->>P: audit_log: transfer.held {rule}
    S-->>U: "Transfer held for review" page

    ST->>S: GET /staff/review (held-transfer queue)
    alt staff approves
        ST->>S: POST /staff/transfers/{id}/release
        S->>P: locked money move (flow 3) → status='completed'
        S->>P: audit_log: transfer.released {reviewer}
    else staff rejects
        ST->>S: POST /staff/transfers/{id}/deny
        S->>P: status='rejected' + reviewer's reason
        S->>P: audit_log: transfer.denied {reviewer}
    end
    Note over ST,P: Every decision is attributable: the audit row records<br/>WHO released/denied WHAT and WHY — the compliance story.
```
