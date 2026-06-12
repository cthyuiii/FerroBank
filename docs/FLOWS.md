# FerroBank - System & Workflow Flow Diagrams

Sequence diagrams for every demonstrable scenario. Render with any Mermaid
viewer (GitHub, VS Code + Mermaid extension, <https://mermaid.live>) - handy
for the report, slides, and the demo recording.

Actors used throughout: **User** (person), **Browser**, **Actix** (middleware +
handler), **Service** (business logic), **PostgreSQL**, **Telegram** (OTP and
notification delivery).

---

## 0. Tech stack - how the systems talk to each other

```mermaid
flowchart LR
    subgraph Client
        U[User] --> B["Browser<br/>SSR HTML + Tailwind CDN + HTMX<br/>toast and status polling"]
    end

    subgraph Server["Actix Web (Rust)"]
        MW["Middleware<br/>session cookie · ActivityGuard<br/>TTL + activity trail + link enforcement · RequireRole"]
        H["Handlers (thin)<br/>parse forms · pick templates"]
        SV["Service traits (thick)<br/>Auth · Account · Transfer · Loan · ActionOtp · Audit · Admin<br/>Arc&lt;dyn Trait&gt; - OOP polymorphism"]
        T["Askama templates<br/>compile-time checked"]
    end

    subgraph Data["PostgreSQL 16"]
        DB[("users · login_sessions · accounts<br/>limit_changes · adjustment_requests<br/>transfers · transfer_reviews · action_otps<br/>loans · loan_approvals · repayments<br/>audit_log · notifications")]
    end

    EXT["Telegram Bot API<br/>OTP + notifications + /unlink commands"]

    B -- "HTTP GET/POST (forms)" --> MW --> H --> SV
    SV -- "SQLx transactions,<br/>SELECT FOR UPDATE" --> DB
    SV -. "audit every state change" .-> DB
    H --> T -- "HTML response" --> B
    SV -. "HTTPS sendMessage" .-> EXT
    EXT -. "getUpdates poller<br/>links accounts, /unlink" .-> SV
    SEED["seed binary"] -- "idempotent demo data" --> DB
    MAIN["main.rs"] -- "migrations on startup ·<br/>system.snapshot on shutdown" --> DB
```

Request path in one line: **Browser → session middleware → ActivityGuard →
RequireRole → handler (thin) → service trait (thick) → SQLx transaction →
PostgreSQL → Askama template → HTML back.** Pages are fully server-side
rendered; the only JSON endpoints are the small polls (notifications, link
status, per-item status).

---

## 1. Registration & login

```mermaid
sequenceDiagram
    actor U as User
    participant A as Actix
    participant S as AuthService
    participant P as PostgreSQL

    U->>A: POST /register with first, middle, last name, NRIC, email, password
    A->>A: validate field lengths, email shape, password >= 8 chars
    A->>S: register NewUser
    S->>S: argon2id hash with unique salt
    S->>P: INSERT users, UNIQUE email
    A-->>U: redirect /login?registered=1, no auto-login
    U->>A: POST /login
    S->>P: verify hash, reset last_activity_at
    alt first-seen device or network for a linked customer
        A-->>U: step-up page, a one-time code goes to Telegram, NO session yet
        U->>A: POST /login/stepup with the code, wrong codes retry inline
        Note over U,A: 3 wrong codes cancel the attempt
        Note over U,A: that browser and network is then blocked for 24 hours
    end
    A-->>U: session cookie + role-based landing, unlinked customers go to Telegram linking
    Note over U,P: identical error for wrong email vs wrong password, no user enumeration
    Note over U,P: known origins stay password-only - the step-up fires only on new origins
```

## 2. Account opening with OTP and maker-checker approval

```mermaid
sequenceDiagram
    actor C as Customer
    actor T as Teller or Admin
    participant A as Actix
    participant P as PostgreSQL

    C->>A: POST /accounts/new, savings or checking
    A->>P: park the request in action_otps, send code via Telegram
    C->>A: POST /accounts/new/confirm with the code
    A->>A: verify, owner-bound, single-use, 10-min expiry, wrong code retries inline
    A->>P: INSERT account status pending, trigger blocks non-customer owners
    Note over C,P: the pending page polls /accounts/id/status and refreshes itself
    T->>A: POST /staff/accounts/id/approve
    A->>P: UPDATE to active, audit, notification + Telegram to the owner
```

## 3. Money transfer - the core engine

```mermaid
sequenceDiagram
    actor U as Customer
    participant A as Actix
    participant S as TransferService
    participant P as PostgreSQL

    U->>A: POST /transfers/new
    A->>A: sender must own the source account
    A->>S: create
    S->>P: recipient must belong to a DIFFERENT user, DB trigger backs this up
    S->>P: insufficient funds pre-check FIRST, records a rejected row, no OTP issued
    S->>P: apply matured limit changes, then enforce the per-transfer limit
    S->>S: Mutex rate limit, 5 per minute per account
    S->>P: INSERT pending + argon2 OTP hash, code goes to Telegram
    U->>A: POST /transfers/confirm
    S->>P: BEGIN, lock transfer FOR UPDATE, must be pending, within 10 minutes
    S->>S: actor must own the source account, verify OTP, 3 strikes rejects
    S->>P: lock both accounts FOR UPDATE in ascending id order, no deadlock
    S->>S: re-check under lock, active status and balance
    alt all checks pass and no fraud rule matches
        S->>P: debit, credit, status completed, one transaction, COMMIT
        S->>P: named notifications both ways + Telegram
    else fraud rule matches
        S->>P: status on_hold with the reason, money untouched
    else a check fails
        S->>P: status rejected with a human-readable reason
    end
```

## 4. Concurrency - why simultaneous transfers cannot corrupt balances

```mermaid
sequenceDiagram
    participant T1 as Transfer task one
    participant T2 as Transfer task two
    participant P as PostgreSQL holding a 15 dollar balance

    par simultaneous confirms of 10 dollars each
        T1->>P: SELECT FOR UPDATE, acquires the row lock
        T2->>P: SELECT FOR UPDATE, BLOCKS waiting for T1
    end
    T1->>P: 15 >= 10, debit, COMMIT, balance now 5
    Note over T2,P: lock released, T2 reads the REAL balance
    T2->>P: 5 >= 10 fails, status rejected, insufficient funds
```

## 5. Fraud hold, identity review, hijack defense

```mermaid
sequenceDiagram
    actor U as Customer
    actor ST as Teller or Admin
    participant S as TransferService
    participant P as PostgreSQL

    U->>S: confirm on a transfer matching a rule
    Note over S: rules - 10k+ large, structuring when a 5k+ balance is emptied to within 49 dollars under the limit, draining over half of a 5k+ balance, 4+ transfers within 1 hour
    S->>P: status on_hold, money NOT moved, named Telegram + toast to the sender
    U->>S: review page, submit purpose and NRIC, page polls status
    ST->>S: /staff/review queue shows the claim beside the NRIC on file
    alt identity verified and purpose plausible
        ST->>S: release, locked re-checks, money moves, both parties told with names
    else suspicious
        ST->>S: deny with a reason, sender told
    end
    Note over S,P: separately - 3 overdraft attempts in 24h freezes the account automatically and messages the owner
```

## 6. Transfer limit change with hold window

```mermaid
sequenceDiagram
    actor U as Customer
    participant A as Actix
    participant S as AccountService
    participant P as PostgreSQL

    U->>A: request a limit change on the account page
    A-->>U: consent popup, increases are held before applying
    U->>A: confirm, then a one-time code page, OTP verified
    alt new limit at or below current
        S->>P: apply immediately, owner notified
    else increase
        S->>P: INSERT limit_changes, effective_at = now + hold window
        Note over S,P: 12 hour hold by default, the OLD limit applies until maturity
        Note over S,P: matured rows are promoted lazily on every account read and before every transfer
        Note over S,P: the page shows the effective moment in the viewer's local time
    end
```

## 7. Loan lifecycle - dual approval, disbursement, due dates

```mermaid
sequenceDiagram
    actor C as Customer
    actor T as Teller
    actor AD as Admin
    participant S as LoanService
    participant P as PostgreSQL

    C->>S: apply with principal, rate, term and a payout account, OTP confirmed
    S->>P: INSERT loan pending, applications expire after 7 days
    T->>S: teller approval, UNIQUE loan and role
    AD->>S: admin approval completes the pair
    S->>P: credit the principal to the payout account in the same transaction
    S->>P: next_payment_due = now + 1 month
    S-->>C: toast with amount and due date, Telegram says open the app to check
    C->>S: repayments debit a chosen funding account under lock
    alt outstanding reaches zero
        S->>P: status paid_off, due date cleared
    else still owing
        S->>P: status active, due date advances a month, customer told the next date
    end
```

## 8. Dual-control balance adjustment

```mermaid
sequenceDiagram
    actor AD as Admin
    actor T as Teller
    participant S as AccountService
    participant P as PostgreSQL

    AD->>S: adjust balance, responsibility popup acknowledged
    alt balance added or deducted at most 1000
        S->>P: apply immediately under a row lock, audited
    else adjustments above 1000
        S->>P: INSERT adjustment_requests, parked
        T->>S: approve as the OTHER staff role, same person or same role is refused
        S->>P: locked apply, refuses a negative result, both ids audited
    end
```

## 9. Profile changes and Telegram linking

```mermaid
sequenceDiagram
    actor U as Customer
    participant A as Actix
    participant T as Telegram
    participant P as PostgreSQL

    U->>A: change email or password, or unlink Telegram
    A->>T: one-time code to the linked chat
    U->>A: confirm with the code, wrong codes retry inline
    A->>P: apply the change, audited
    Note over U,T: bot commands - /start code links, /help lists commands
    Note over U,T: /unlink is hardened - it schedules the unlink 24h out with warnings on every channel
    Note over U,T: cancellable only from a password-backed website session
    Note over U,T: the website's own OTP-confirmed unlink stays immediate
```

## 10. Sessions, TTLs, and the activity trail

```mermaid
sequenceDiagram
    participant B as Browser
    participant G as ActivityGuard middleware
    participant P as PostgreSQL

    B->>G: any request with a session
    G->>G: log user id, method, path - the per-user activity trail
    G->>P: last_activity_at older than 5 minutes on DATABASE time?
    alt expired
        G-->>B: session purged, redirect /login?expired=1 with the reason shown
    else active
        G->>P: UPDATE last_activity_at = now
        G-->>B: request proceeds
    end
    Note over B,P: the cookie key mixes in a per-boot nonce, so a server restart logs everyone out
    Note over B,P: other TTLs - OTPs and pending transfers 10 minutes, pending loans 7 days
```

## 11. Notifications and live status

```mermaid
sequenceDiagram
    participant SV as Any service
    participant P as PostgreSQL
    participant B as Browser
    participant T as Telegram

    SV->>P: INSERT notifications on approvals, transfers, holds, releases, denials, freezes, limit changes, loan decisions
    SV->>T: mirrored to Telegram when the user is linked
    loop every 5 seconds and on tab focus
        B->>P: GET /notifications, fetch-and-mark-seen
        B-->>B: toast per message, fires exactly once
    end
    loop every 4 seconds on pending pages
        B->>P: GET status for a held transfer, pending account, or pending loan
        B-->>B: auto-redirect or refresh the moment staff act
    end
```

## 12. Graceful shutdown - system snapshot

```mermaid
sequenceDiagram
    participant OS as Ctrl-C or SIGTERM
    participant M as main.rs
    participant P as PostgreSQL

    OS->>M: signal, actix finishes in-flight requests
    M->>P: dedicated connection, counts and totals across the bank
    M->>P: INSERT audit_log event system.snapshot with the JSON payload
    Note over M,P: a hard crash cannot run anything, but committed transactions are already durable via the WAL
    Note over M,P: the snapshot is a forensic marker, not recovery
```

## 13. Login device tracking

```mermaid
sequenceDiagram
    actor U as User
    participant A as Actix
    participant P as PostgreSQL
    participant T as Telegram

    U->>A: POST /login from some browser and network
    A->>A: classify the User-Agent - Chrome, Safari, Firefox, Edge, Other
    A->>P: compare against this user's login history
    A->>P: INSERT login_sessions with browser, IP, first-seen flags
    alt first-seen browser or first-seen network
        A->>P: audit auth.login.new_origin + notification
        A->>T: security alert - new device or network, change your password if not you
    end
    Note over A,P: staff see the full history with New device and New network badges on /staff/users/id
    Note over A,P: shown beside the user's accounts and transfers - pair with held transfers when investigating takeovers
```
