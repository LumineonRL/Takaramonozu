# Statistical comparison between the two simulators to validate the outputs
# match where they're supposed to match.
#
# The big thing about scores in SIF are that the scores that a team produces
# need not be normal, *especially* with the coinflip mechanic on amp/encore teams.
# For that reason, it's important to use statistical tests that do not assume normality.
# 
# Although the results here don't really show a bimodal result for amp+encore
# like I was expecting, but it did produce a right skew distribution which I guess
# makes sense. In fact, several of the teams lead to some pretty heavily skew distributions.
# One interesting takeaway was tail behavior was quite well-behaved on every spec
# I looked at. Pretty much perfectly normal at the tails on every spec.
#
# So the things this script looks at to compare the two simulators are
# A) The mean
# B) The shape with a 2-sample Kolmogorov-Smirnov test
# C) The speed. In microseconds. Isn't it fitting that a Love Live simulator
# is measured in μ's?
#
# I guess a few notes on speed.
# C-1) Both single-threaded because that's what burrito does.
# C-2) To be fair to Burrito, let it warm up.
# So I actually do two runs with Burrito, one with significantly less trials (one-fifth, to be precise)
# and take the slope bewteen them.
#
# Normally I'd have the tidyverse but since this isn't an R project and rather a quick script I threw together
# for an ad hoc analysis and I didn't want to increase the barrier of entry for any of the 0 people that
# will ever try to reproduce this script, everything is just in base R. Which was only moderately
# annoying for some of the string operations everything else was quite natural.
#
# Usage:
#     Rscript bench/compare.R
#     Rscript bench/compare.R scorers star-notes
#     Rscript bench/compare.R --trials 20000 --rounds 5

script <- grep("^--file=", commandArgs(), value = TRUE) |> sub(pattern = "^--file=", replacement = "")
ROOT <- script |> normalizePath() |> dirname() |> dirname()
SPECS <- file.path(ROOT, "bench/specs")
REFERENCE <- file.path(ROOT, "bench/reference")
TAKARAMONOZU <- file.path(ROOT, "target/release/runner")
BURRITO <- file.path(REFERENCE, "dist/runner.mjs")
PERCENTILES <- c(1, 50, 99)
WARMUP_SHARE <- 5

arguments <- function(argv) {
    o <- list(specs = character(), trials = 4000L, rounds = 3L, seed = 315L, build = TRUE)
    i <- 1
    while (i <= length(argv)) {
        a <- argv[i]
        if (a %in% c("--trials", "--rounds", "--seed")) {
            value <- suppressWarnings(as.integer(argv[i + 1]))
            if (is.na(value)) stop(a, " needs a whole number")
            o[[sub("^--", "", a)]] <- value
            i <- i + 1
        } else if (a == "--no-build") {
            o$build <- FALSE
        } else if (startsWith(a, "--")) {
            stop("unknown option ", a,
                 "\nusage: compare.R [spec ...] [--trials N] [--rounds N] [--seed N] [--no-build]")
        } else {
            o$specs <- c(o$specs, a)
        }
        i <- i + 1
    }
    o
}

run <- function(command, args) {
    status <- system2(command, args)
    if (status != 0) stop(command, " exited ", status)
}

build <- function() {
    message("building both runners")
    run("cargo", c("build", "--release", "-p", "bench", "--bin", "runner"))
    run("npm", c("run", "-s", "build", "--prefix", shQuote(REFERENCE), ">", "/dev/null"))
}

comparable <- function() {
    list.files(SPECS, pattern = "\\.json$", full.names = TRUE) |>
        sort() |>
        Filter(f = \(p) {
            # I miss stringr
            readLines(p) |>
                paste(collapse = "\n") |>
                grepl(pattern = '"run_burrito"[[:space:]]*:[[:space:]]*true')
        }) |>
        basename() |>
        sub(pattern = "\\.json$", replacement = "")
}

capture <- function(json, pattern, what) {
    hit <- regmatches(json, regexec(pattern, json))[[1]]
    if (length(hit) < 2) stop("no ", what, " in runner output")
    hit[2]
}

number <- function(json, name) {
    capture(json, sprintf('"%s":[[:space:]]*([[:digit:]]+)', name), name) |> as.numeric()
}

read_run <- function(json) {
    listed <- capture(json, '"scores":[[:space:]]*\\[([[:digit:].,[:space:]]*)\\]', "scores")
    list(
        json = json,
        trials = number(json, "trials"),
        trials_ns = number(json, "trials_ns"),
        scores = listed |> strsplit("[[:space:]]*,[[:space:]]*") |> unlist() |> as.numeric()
    )
}

play <- function(runner, spec, trials, seed) {
    args <- c(runner[-1], shQuote(file.path(SPECS, paste0(spec, ".json"))), trials, seed)
    out <- system2(runner[1], args, stdout = TRUE)
    status <- attr(out, "status")
    if (!is.null(status)) stop(spec, ": ", basename(runner[length(runner)]), " exited ", status)
    out |> paste(collapse = "\n") |> read_run()
}

takaramonozu <- function(spec, trials, seed) play(TAKARAMONOZU, spec, trials, seed)
burrito <- function(spec, trials, seed) play(c("node", shQuote(BURRITO)), spec, trials, seed)

per_trial_micros <- function(r) r$trials_ns / r$trials / 1000
slope_micros <- function(short, long) (long$trials_ns - short$trials_ns) / (long$trials - short$trials) / 1000
gap <- function(a, b) (a - b) / b * 100

compare <- function(name, a, b, a_micros, b_micros) {
    x <- a$scores
    y <- b$scores
    se <- sqrt(var(x) / length(x) + var(y) / length(y))
    diff <- mean(x) - mean(y)

    ks <- ks.test(x, y, exact = TRUE)
    tails <- gap(quantile(x, PERCENTILES / 100), quantile(y, PERCENTILES / 100))
    data.frame(
        spec = name,
        takaramonozu = mean(x),
        burrito = mean(y),
        gap = gap(mean(x), mean(y)),
        z = if (se > 0) diff / se else if (diff == 0) 0 else Inf,
        ks_d = unname(ks$statistic),
        ks_p = ks$p.value,
        p1 = tails[1],
        p50 = tails[2],
        p99 = tails[3],
        takaramonozu_micros = a_micros,
        burrito_micros = b_micros,
        row.names = NULL
    )
}

# Entirely AI-written, LLMs are really damn fast at this sort of things.
markdown <- function(rows, trials, rounds) {
    signed <- function(x, digits) sprintf(paste0("%+.", digits, "f"), x)
    commas <- function(x) formatC(round(x), format = "d", big.mark = ",")
    tails <- paste(signed(rows$p1, 2), signed(rows$p50, 2), signed(rows$p99, 2), sep = "% / ")
    body <- sprintf(
        "| %s | %s | %s | %s%% | %s | %.4f | %.3f | %s%% | %.2f | %.1f | %.0f× |",
        rows$spec, commas(rows$takaramonozu), commas(rows$burrito), signed(rows$gap, 3), signed(rows$z, 1),
        rows$ks_d, rows$ks_p, tails, rows$takaramonozu_micros, rows$burrito_micros, rows$burrito_micros / rows$takaramonozu_micros
    )
    c(
        sprintf("%d trials, speed is min of %d round(s)", trials, rounds),
        "",
        paste("| spec | takaramonozu mean | burrito mean | gap | z | KS D | KS p | p1 / p50 / p99 gap",
              "| takaramonozu µs | burrito µs | takaramonozu faster |"),
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        body
    )
}

main <- function() {
    o <- arguments(commandArgs(trailingOnly = TRUE))
    setwd(ROOT)
    specs <- if (length(o$specs)) o$specs else comparable()
    if (o$build) build()
    out <- file.path(ROOT, "bench/results", format(Sys.time(), "%Y-%m-%dT%H-%M-%S"))
    dir.create(out, recursive = TRUE)

    short <- max(o$trials %/% WARMUP_SHARE, 1)
    runs <- list()
    takaramonozu_micros <- burrito_micros <- setNames(vector("list", length(specs)), specs)
    for (lap in seq_len(o$rounds)) {
        for (name in specs) {
            message(sprintf("round %d/%d: %s", lap, o$rounds, name))
            a <- takaramonozu(name, o$trials, o$seed)
            b_short <- burrito(name, short, o$seed)
            b <- burrito(name, o$trials, o$seed)
            takaramonozu_micros[[name]] <- c(takaramonozu_micros[[name]], per_trial_micros(a))
            burrito_micros[[name]] <- c(burrito_micros[[name]], slope_micros(b_short, b))
            if (is.null(runs[[name]])) runs[[name]] <- list(takaramonozu = a, burrito = b)
        }
    }

    rows <- specs |>
        lapply(\(name) {
            a <- runs[[name]]$takaramonozu
            b <- runs[[name]]$burrito
            writeLines(a$json, file.path(out, paste0(name, ".takaramonozu.json")))
            writeLines(b$json, file.path(out, paste0(name, ".burrito.json")))
            compare(name, a, b, min(takaramonozu_micros[[name]]), min(burrito_micros[[name]]))
        }) |>
        do.call(what = rbind)
    write.csv(rows, file.path(out, "summary.csv"), row.names = FALSE)

    writeLines(markdown(rows, o$trials, o$rounds))
    message(sprintf("median: takaramonozu %.0f× faster", median(rows$burrito_micros / rows$takaramonozu_micros)))
    message("results in ", sub(paste0(ROOT, "/"), "", out, fixed = TRUE))
}

main()
