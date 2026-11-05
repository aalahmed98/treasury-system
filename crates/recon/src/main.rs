//! treasury-recon: the reconciliation worker, the synthetic feed, and maintenance commands.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::{Context, bail};
use chrono::{DateTime, NaiveDate, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use domain::Source;
use recon::calendar::Calendar;
use recon::clock::{Clock, SimClock, SystemClock};
use recon::pipeline::{Engine, EngineConfig};
use recon::synth::{Emission, SynthConfig, TrueDeal, World, user_id};
use sqlx::PgPool;
use sqlx::postgres::{PgListener, PgPoolOptions};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "treasury-recon", about = "Reconciliation worker and synthetic feeds")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the worker: process the event log as it grows, check SLAs, publish status.
    Serve,
    /// Generate a synthetic world and feed it through the pipeline, running the worker in-process.
    Synth {
        /// TOML file; the built-in default is used when omitted.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Mode::Replay)]
        mode: Mode,
        /// Replay speed relative to wall time.
        #[arg(long, default_value_t = 600.0)]
        speed: f64,
        #[arg(long)]
        days: Option<u32>,
        #[arg(long)]
        start: Option<NaiveDate>,
        #[arg(long)]
        seed: Option<u64>,
        /// Write the generated emissions as JSON and exit.
        #[arg(long)]
        dump: Option<PathBuf>,
        /// After feeding everything, keep serving instead of exiting.
        #[arg(long)]
        keep_serving: bool,
    },
    /// Wipe the projection (links, breaks, external deals) and replay the whole log.
    Rebuild,
    /// Generate synthetic nostro statements and movements for the expected flows in a date range.
    Nostro {
        #[arg(long)]
        from: NaiveDate,
        #[arg(long)]
        to: NaiveDate,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Generate the synthetic banking book, curves and spots for the ALM workbench.
    AlmSynth {
        /// Business date of the newest snapshot; defaults to the reconciler's clock, else today.
        #[arg(long)]
        date: Option<NaiveDate>,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// TOML file with an [alm] section; the built-in default is used when omitted.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Queue depth, feed freshness, deals by match status, open breaks by kind.
    Status,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Mode {
    /// Simulated clock from the world's first event, at --speed.
    Replay,
    /// Everything up to now at once, then live for anything later.
    CatchUp,
    /// Wall clock; a world for today, fed as it happens.
    Realtime,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()))
        .init();
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        warn!("a TLS crypto provider was already installed");
    }
    match Cli::parse().cmd {
        Cmd::Serve => serve().await,
        Cmd::Synth { config, mode, speed, days, start, seed, dump, keep_serving } => {
            synth(config, mode, speed, days, start, seed, dump, keep_serving).await
        }
        Cmd::Rebuild => rebuild().await,
        Cmd::Nostro { from, to, seed } => nostro(from, to, seed).await,
        Cmd::AlmSynth { date, seed, config } => alm_synth(date, seed, config).await,
        Cmd::Status => status().await,
    }
}

async fn recon_pool() -> anyhow::Result<PgPool> {
    let url = std::env::var("RECON_DATABASE_URL").context("RECON_DATABASE_URL is required")?;
    PgPoolOptions::new().max_connections(8).connect(&url).await.context("connecting as the reconciler role")
}

/// Only one worker may run against a database. The lock lives on a connection we keep.
async fn singleton(pool: &PgPool) -> anyhow::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let mut conn = pool.acquire().await?;
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext('treasury-recon'))").fetch_one(&mut *conn).await?;
    if !got {
        bail!("another treasury-recon worker holds the lock on this database");
    }
    Ok(conn)
}

async fn calendar(pool: &PgPool) -> anyhow::Result<Calendar> {
    let holidays: Vec<NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(pool).await?;
    Ok(Calendar::new(holidays))
}

// ---------------------------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------------------------

async fn serve() -> anyhow::Result<()> {
    let pool = recon_pool().await?;
    let _lock = singleton(&pool).await?;
    let engine = Arc::new(Engine::load(pool.clone(), Arc::new(SystemClock), EngineConfig::default()).await?);
    info!("treasury-recon worker started");
    run_worker(engine, pool, StdDuration::from_secs(10), StdDuration::from_secs(5), tokio::signal::ctrl_c()).await
}

/// Process on notification or poll, tick SLAs on an interval, publish a heartbeat, until `stop`.
async fn run_worker(
    engine: Arc<Engine>,
    pool: PgPool,
    sla_every: StdDuration,
    heartbeat_every: StdDuration,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen("source_events_new").await?;
    let mut sla = tokio::time::interval(sla_every);
    let mut hb = tokio::time::interval(heartbeat_every);
    let mut poll = tokio::time::interval(StdDuration::from_secs(1));
    // A slow tick must not be followed by a burst of catch-up ticks.
    sla.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => { info!("stopping"); return Ok(()); }
            r = listener.try_recv() => {
                if let Err(e) = r { warn!(error = %e, "listener"); }
                drain(&engine).await;
            }
            _ = poll.tick() => drain(&engine).await,
            _ = sla.tick() => {
                drain(&engine).await;
                match engine.sla_tick().await {
                    Ok(r) if r.missing_raised + r.orphans_raised > 0 || !r.stale_feeds.is_empty() => {
                        info!(missing = r.missing_raised, orphans = r.orphans_raised, stale = ?r.stale_feeds, pending = r.pending, "sla tick");
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "sla tick failed"),
                }
            }
            _ = hb.tick() => {
                if let Err(e) = engine.heartbeat().await { warn!(error = %e, "heartbeat"); }
            }
        }
    }
}

async fn drain(engine: &Engine) {
    match engine.process_until_idle().await {
        Ok(n) if n > 0 => tracing::debug!(events = n, "processed"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "processing failed"),
    }
}

// ---------------------------------------------------------------------------------------------
// synth
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn synth(
    config: Option<PathBuf>,
    mode: Mode,
    speed: f64,
    days: Option<u32>,
    start: Option<NaiveDate>,
    seed: Option<u64>,
    dump: Option<PathBuf>,
    keep_serving: bool,
) -> anyhow::Result<()> {
    let mut cfg: SynthConfig = match config {
        Some(p) => toml::from_str(&std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?)?,
        None => SynthConfig::default(),
    };
    if let Some(d) = days {
        cfg.world.days = d;
    }
    if let Some(s) = seed {
        cfg.world.seed = s;
    }
    if let Some(s) = start {
        cfg.world.start = s;
    } else if mode == Mode::Realtime {
        cfg.world.start = Utc::now().date_naive();
        cfg.world.days = days.unwrap_or(1);
    }

    let pool = recon_pool().await?;
    let app_url = std::env::var("APP_DATABASE_URL").context("APP_DATABASE_URL is required for internal bookings")?;
    let app = PgPoolOptions::new().max_connections(4).connect(&app_url).await.context("connecting as the booking role")?;
    let cal = calendar(&pool).await?;

    let world = World::generate(cfg, &cal);
    let emissions = world.emit(&cal);
    let deals_by_scenario = world.deals.iter().fold(std::collections::BTreeMap::<String, usize>::new(), |mut m, d| {
        *m.entry(format!("{:?}", d.scenario)).or_default() += 1;
        m
    });
    info!(deals = world.deals.len(), emissions = emissions.len(), scenarios = ?deals_by_scenario, "world generated");
    if let Some(p) = dump {
        std::fs::write(&p, serde_json::to_vec_pretty(&emissions)?)?;
        info!(path = %p.display(), "emissions written");
        return Ok(());
    }

    let _lock = singleton(&pool).await?;
    let first = emissions.first().map(Emission::at).unwrap_or_else(Utc::now);
    let last = emissions.last().map(Emission::at).unwrap_or(first);
    let sim: Option<Arc<SimClock>> = (mode == Mode::Replay).then(|| Arc::new(SimClock::new(first, speed)));
    let clock: Arc<dyn Clock> = match &sim {
        Some(s) => s.clone(),
        None => Arc::new(SystemClock),
    };
    let engine = Arc::new(Engine::load(pool.clone(), clock.clone(), EngineConfig::default()).await?);
    let unassigned = engine.unassigned_desk();

    // The worker runs alongside the feeder; SLA ticks come often in replay so detections land
    // at roughly the right simulated time.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let worker = {
        let engine = engine.clone();
        let pool = pool.clone();
        let sla_every = if mode == Mode::Replay {
            StdDuration::from_millis(((300.0 / speed.max(1.0)) * 1000.0).clamp(50.0, 10_000.0) as u64)
        } else {
            StdDuration::from_secs(10)
        };
        tokio::spawn(async move {
            let stop = async move {
                let _ = stop_rx.await;
                Ok(())
            };
            run_worker(engine, pool, sla_every, StdDuration::from_secs(2), stop).await
        })
    };

    let total = emissions.len();
    let mut fed = 0usize;
    let mut log_at = std::time::Instant::now();
    for e in emissions {
        let at = e.at();
        wait_until(&*clock, at, if mode == Mode::Replay { speed } else { 1.0 }, mode).await;
        match e {
            Emission::Event { source, source_ref, kind, seq, source_ts, payload, .. } => {
                let inserted = recon::ingest::insert_event(&pool, source, &source_ref, kind, seq, source_ts, at, &payload, unassigned).await?;
                if inserted.is_some() {
                    sqlx::query("UPDATE feed_status SET last_event_at = $2, last_source_ts = $3, events_total = events_total + 1 WHERE source = $1")
                        .bind(source.as_str())
                        .bind(at)
                        .bind(source_ts)
                        .execute(&pool)
                        .await?;
                } else {
                    sqlx::query("UPDATE feed_status SET duplicates_dropped = duplicates_dropped + 1 WHERE source = $1")
                        .bind(source.as_str())
                        .execute(&pool)
                        .await?;
                }
            }
            Emission::Internal { deal, .. } => {
                if let Some(d) = world.deals.iter().find(|d| d.n == deal) {
                    if let Err(err) = book_internal(&app, d, at).await {
                        warn!(deal, error = %err, "internal booking failed");
                    }
                }
            }
            Emission::Heartbeat { source, .. } => {
                sqlx::query("UPDATE feed_status SET last_heartbeat_at = $2 WHERE source = $1").bind(source.as_str()).bind(at).execute(&pool).await?;
            }
        }
        fed += 1;
        if log_at.elapsed() > StdDuration::from_secs(5) {
            info!(fed, total, sim_now = %clock.now(), "feeding");
            log_at = std::time::Instant::now();
        }
    }
    info!(fed, "feed complete; settling");
    // Freeze simulated time just after the last emission so the final checks reflect that moment,
    // then stop the in-process worker before running them ourselves (no concurrent ticks).
    if let Some(s) = &sim {
        s.pause(Some(last + chrono::Duration::minutes(5)));
    }
    let _ = stop_tx.send(());
    let _ = worker.await;
    engine.process_until_idle().await?;
    let r = engine.sla_tick().await?;
    info!(missing = r.missing_raised, orphans = r.orphans_raised, pending = r.pending, clock = %clock.now(), "final sla tick");
    // What the correspondent banks reported for the cash those deals moved, up to the simulated present.
    let n = recon::nostro::generate(&pool, &cal, world.cfg.world.start, world.window_end.date_naive(), world.cfg.world.seed).await?;
    info!(statements = n.statements, movements = n.movements, missing = n.missing, unexpected = n.unexpected, adjusted = n.adjusted, "nostro statements generated");
    if world.cfg.alm.enabled {
        let a = recon::alm_synth::run(&pool, &cal, &world.cfg.alm, world.window_end.date_naive(), world.cfg.world.seed).await?;
        info!(snapshots = a.snapshots, skipped = a.skipped, positions = a.positions, depositors = a.depositors, rates = a.rates, spots = a.spots, periods = a.periods, ftp = a.ftp, "banking book generated");
    }
    print_summary(&pool).await?;

    if keep_serving {
        info!("keep-serving: worker stays up on the wall clock (Ctrl-C to stop)");
        let live = Arc::new(Engine::load(pool.clone(), Arc::new(SystemClock), EngineConfig::default()).await?);
        run_worker(live, pool.clone(), StdDuration::from_secs(10), StdDuration::from_secs(5), tokio::signal::ctrl_c()).await?;
    }
    Ok(())
}

/// Sleep until the clock reaches `at`. In replay the clock runs at `speed`; in catch-up and
/// realtime it is the wall clock, so past emissions go through immediately.
async fn wait_until(clock: &dyn Clock, at: DateTime<Utc>, speed: f64, mode: Mode) {
    loop {
        let now = clock.now();
        if now >= at {
            return;
        }
        let gap = (at - now).num_milliseconds().max(0) as f64;
        let wall_ms = if mode == Mode::Replay { gap / speed.max(0.001) } else { gap };
        tokio::time::sleep(StdDuration::from_millis(wall_ms.min(250.0).max(1.0) as u64)).await;
    }
}

/// Book a true deal as its trader through the booking role, exactly as the API would.
async fn book_internal(app: &PgPool, d: &TrueDeal, at: DateTime<Utc>) -> anyhow::Result<uuid::Uuid> {
    let mut tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)").bind(user_id(d.trader).to_string()).execute(&mut *tx).await?;
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, maturity_date, \
                            counterparty, trader_id, status, created_at, updated_at) \
         SELECT gen_random_uuid(), $1 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), id, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                $11, $12, 'BOOKED', $13, $13 \
         FROM desks WHERE code = $1 RETURNING id",
    )
    .bind(d.desk)
    .bind(d.product.as_str())
    .bind(d.side.as_str())
    .bind(&d.ccy1)
    .bind(&d.ccy2)
    .bind(d.notional)
    .bind(d.rate)
    .bind(d.trade_date)
    .bind(d.value_date)
    .bind(d.maturity_date)
    .bind(&d.counterparty)
    .bind(user_id(d.trader))
    .bind(at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

// ---------------------------------------------------------------------------------------------
// rebuild / status
// ---------------------------------------------------------------------------------------------

async fn rebuild() -> anyhow::Result<()> {
    let pool = recon_pool().await?;
    let _lock = singleton(&pool).await?;
    let engine = Engine::load(pool.clone(), Arc::new(SystemClock), EngineConfig::default()).await?;
    let n = engine.rebuild().await?;
    let r = engine.sla_tick().await?;
    info!(replayed = n, missing = r.missing_raised, orphans = r.orphans_raised, "rebuild complete");
    let stale: i64 = sqlx::query_scalar("SELECT count(*) FROM source_events WHERE source = 'USER' AND outcome = 'STALE'").fetch_one(&pool).await?;
    if stale > 0 {
        warn!(stale, "user actions could not be replayed (their breaks were re-raised with new ids); review open breaks");
    }
    print_summary(&pool).await
}

async fn nostro(from: NaiveDate, to: NaiveDate, seed: u64) -> anyhow::Result<()> {
    let pool = recon_pool().await?;
    let cal = calendar(&pool).await?;
    let n = recon::nostro::generate(&pool, &cal, from, to, seed).await?;
    info!(statements = n.statements, movements = n.movements, missing = n.missing, unexpected = n.unexpected, adjusted = n.adjusted, "nostro statements generated");
    Ok(())
}

async fn alm_synth(date: Option<NaiveDate>, seed: u64, config: Option<PathBuf>) -> anyhow::Result<()> {
    let cfg: SynthConfig = match config {
        Some(p) => toml::from_str(&std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?)?,
        None => SynthConfig::default(),
    };
    let pool = recon_pool().await?;
    let cal = calendar(&pool).await?;
    let date = match date {
        Some(d) => d,
        None => {
            let clock: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT now FROM recon_status WHERE name = 'worker'").fetch_optional(&pool).await?;
            clock.unwrap_or_else(Utc::now).date_naive()
        }
    };
    let a = recon::alm_synth::run(&pool, &cal, &cfg.alm, date, seed).await?;
    info!(business_date = %date, snapshots = a.snapshots, skipped = a.skipped, positions = a.positions, depositors = a.depositors, rates = a.rates, spots = a.spots, periods = a.periods, ftp = a.ftp, "banking book generated");
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let pool = recon_pool().await?;
    let st: Option<(DateTime<Utc>, DateTime<Utc>, i64)> =
        sqlx::query_as("SELECT now, last_tick_at, pending FROM recon_status WHERE name = 'worker'").fetch_optional(&pool).await?;
    match st {
        Some((now, tick, pending)) => println!("worker clock {now}  last tick {tick}  pending {pending}"),
        None => println!("worker has never reported"),
    }
    let feeds: Vec<(String, Option<DateTime<Utc>>, Option<DateTime<Utc>>, i64, i64)> =
        sqlx::query_as("SELECT source, last_heartbeat_at, last_event_at, events_total, duplicates_dropped FROM feed_status ORDER BY source")
            .fetch_all(&pool)
            .await?;
    for (s, hb, ev, n, d) in feeds {
        println!("{s:<10} heartbeat {:<26} last event {:<26} events {n:>6} dups {d}", opt(hb), opt(ev));
    }
    print_summary(&pool).await
}

fn opt(t: Option<DateTime<Utc>>) -> String {
    t.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_else(|| "-".into())
}

async fn print_summary(pool: &PgPool) -> anyhow::Result<()> {
    let deals: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT match_status, count(*), count(*) FILTER (WHERE confirmed) FROM deals WHERE merged_into IS NULL GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;
    println!("deals by match status (confirmed):");
    for (m, n, c) in deals {
        println!("  {m:<10} {n:>6}  ({c} confirmed)");
    }
    let brks: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT kind, status, count(*) FROM breaks GROUP BY 1, 2 ORDER BY 1, 2").fetch_all(pool).await?;
    println!("breaks:");
    for (k, s, n) in brks {
        println!("  {k:<10} {s:<12} {n:>6}");
    }
    let outcomes: Vec<(String, i64)> =
        sqlx::query_as("SELECT coalesce(outcome, 'PENDING'), count(*) FROM source_events GROUP BY 1 ORDER BY 2 DESC").fetch_all(pool).await?;
    println!("event outcomes: {}", outcomes.iter().map(|(o, n)| format!("{o}={n}")).collect::<Vec<_>>().join("  "));
    let _ = Source::Kondor; // keep the import meaningful for future per-source output
    Ok(())
}
