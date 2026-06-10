# FerroBank — System & Workflow Flow Diagrams

Sequence/flow diagrams for every demonstrable scenario, core workflows first,
then the security and verification features built on top of them.
Render with any Mermaid viewer (GitHub, VS Code + Mermaid extension,
<https://mermaid.live>) — handy for the report, slides, and the demo recording.

Actors used throughout: **User** (person), **Browser**, **Actix** (middleware +
handler), **Service** (business logic), **PostgreSQL**, plus **Telegram** for
the OTP/notification delivery channel.

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
    participant A as Actix auth handler
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
    participant A as Actix transfers handler
    participant S as TransferService
    participant P as PostgreSQL

    U->>B: transfer form (from, to acct number, amount)
    B->>A: POST /transfers/new
    A->>A: parse Decimal · sender must OWN from-account
    A->>S: create(actor, from, to, amount)
    S->>P: recipient must belong to a DIFFERENT user<br/>(no self-transfers; DB trigger backs this up)
    S->>P: apply matured limit changes · enforce per-transfer limit
    S->>S: Mutex rate-limit (≤5/min per account)
    S->>S: generate 6-digit OTP → argon2 hash
    S->>P: INSERT transfer status='pending' + otp_hash
    S->>P: audit_log: transfer.created
    S-->>B: confirm page (code goes to Telegram — never on screen when linked)

    U->>B: types OTP
    B->>A: POST /transfers/confirm
    A->>S: confirm(actor, transfer_id, otp)
    S->>P: BEGIN
    S->>P: SELECT transfer FOR UPDATE (must be 'pending')
    S->>P: actor must own the source account (else Forbidden)
    S->>S: verify OTP against argon2 hash (3 strikes → rejected · 10-min TTL)
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
    participant T1 as Transfer task 1 of 10 dollars
    participant T2 as Transfer task 2 of 10 dollars
    participant P as PostgreSQL with balance 15

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
    participant OS as OS signal Ctrl-C or SIGTERM
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

# Security & verification features

## 7. OTP delivery via Telegram (mandatory for customers)

```mermaid
sequenceDiagram
    actor U as Customer
    participant B as Browser
    participant A as Actix
    participant TG as Telegram Bot API
    participant P as PostgreSQL

    Note over U,P: After registration the customer signs in and is routed to /settings/telegram and nowhere else until linked
    U->>B: tap "Open Telegram & link my account" (single-use code in deep link)
    U->>TG: press Start → /start code
    TG-->>A: getUpdates poll delivers code + chat id
    A->>P: consume code, store chat id
    Note over B: the guide page polls /settings/telegram/status every 2s and refreshes itself the moment the link lands
    B-->>U: "Linked ✓ — codes & account updates arrive in Telegram"

    Note over U,P: Every sensitive action afterwards (transfer, account opening, loan application, profile change) sends its code to Telegram — codes never appear on screen for linked users. Bot commands: /unlink (clears the link in the DB), /help.
```

## 8. Fraud hold + staff review (NRIC identity check)

```mermaid
sequenceDiagram
    actor U as Customer
    actor ST as Teller or Admin
    participant S as TransferService
    participant P as PostgreSQL

    U->>S: confirm(...) on a transfer matching a fraud rule
    Note over S: rules: ≥$10k large · $9k–10k structuring · drains >50% of a balance >$5k · 4+ transfers within 1 hour · (3 invalid OTP attempts rejects outright)
    S->>P: status='on_hold' + reason — money NOT moved
    S-->>U: review page: "this may be illegitimate" → submit purpose + NRIC
    U->>S: submit_review(purpose, NRIC claim)
    S->>P: INSERT transfer_reviews

    ST->>S: GET /staff/review — queue shows claim vs NRIC on file
    alt identity verified, purpose plausible
        ST->>S: release → locked re-checks → money moves → 'completed'
    else suspicious
        ST->>S: deny(reason) → 'rejected'
    end
    S->>P: audit_log + notification (+ Telegram message) either way

    Note over S,P: Hijack heuristic: 3+ overdraft attempts from one account in 24h freezes the account automatically.
```

## 9. Transfer limit change with hold window

```mermaid
sequenceDiagram
    actor U as Customer
    participant B as Browser
    participant S as AccountService
    participant P as PostgreSQL

    U->>B: account page → request limit change
    B->>U: consent popup: increases are risky → held before applying
    U->>B: confirm
    alt new limit ≤ current
        S->>P: apply immediately (lowering reduces risk)
    else increase
        S->>P: INSERT limit_changes, effective_at = now() + hold window
        Note over S,P: 12h default · Alice's accounts seeded to 10s for the demo. The OLD limit applies until maturity; the transfer engine promotes matured rows lazily before enforcing.
    end
```

## 10. Inactivity TTL + activity trail (every request)

```mermaid
sequenceDiagram
    participant B as Browser
    participant G as ActivityGuard middleware
    participant P as PostgreSQL

    B->>G: any GET/POST/PUT/DELETE with a session
    G->>G: log user id + method + path (traceable activity)
    G->>P: last_activity_at older than 5 minutes? (database time)
    alt expired
        G->>G: purge session
        G-->>B: 302 /login?expired=1 — "signed out for inactivity"
    else active
        G->>P: UPDATE last_activity_at = now()
        G-->>B: request proceeds
    end
    Note over G,P: TTLs everywhere: sessions 5 min idle · OTPs 10 min · pending transfers 10 min · pending loans 7 days · notifications fire once.
```

## 11. Notifications (browser toasts + Telegram)

```mermaid
sequenceDiagram
    participant SV as Any service
    participant P as PostgreSQL
    participant B as Browser (layout.html)
    participant TG as Telegram

    SV->>P: INSERT notifications (account approved, transfer completed/held/denied, loan approved/declined, account frozen)
    SV->>TG: sendMessage when the user is linked
    loop every 8 seconds
        B->>P: GET /notifications (fetch-and-mark-seen)
        B-->>B: toast popup per message — fires exactly once
    end
```
