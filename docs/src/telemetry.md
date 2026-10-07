---
title: Telemetry
description: "What this fork collects and how to control crash reporting."
---

# Telemetry in Zed

This fork does not upload usage metrics or talk to any Zed-hosted service. The
analytics uploader was removed: there is no event queue, no
`POST /telemetry/events` request, and `telemetry::event!(...)` calls throughout
the codebase are compiled no-ops.

What remains is **crash reporting**, which you can turn off in settings.

## Configuring Telemetry Settings

Open Settings ({#kb zed::OpenSettings}) and search for "telemetry", or add the following to your settings file:

```json [settings]
"telemetry": {
    "diagnostics": false,
    "metrics": false
},
```

- `diagnostics` controls crash reporting (minidumps).
- `metrics` is accepted so existing settings files keep parsing. It has no effect, because no metrics are uploaded.

## Crash Reporting {#diagnostics}

Crash reports consist of a [minidump](https://learn.microsoft.com/en-us/windows/win32/debug/minidump-files) and debug metadata. Reports are sent on the next launch after a crash, so problems can be identified without you filing a bug report. Installations built without a minidump endpoint (the default for local development builds) do not send anything.

You can inspect what data is sent in the `CrashInfo` struct in [crates/crashes/src/crashes.rs](https://github.com/zed-industries/zed/blob/main/crates/crashes/src/crashes.rs). See also: [Debugging Crashes](./development/debugging-crashes.md).

## Installation and System Identifiers {#identifiers}

Zed still generates an installation id and a system id locally. They are used to
group crash reports: the crash reporter tags reports with
`SENTRY_USER_ID = installation-<id>`. Nothing signs in, and no account id is
attached to them.

System and OS details (OS name, OS version, architecture, app version) are
included with crash reports.

## Removed in This Fork {#removed}

- Client-side metrics (file extensions, features used, project statistics, detected frameworks)
- Server-side metrics for hosted services (rate limiting, billing, token usage)
- The telemetry log viewer (`Help > View Telemetry Log` and the `zed::OpenTelemetryLog` action)
- Org-wide data-sharing controls for Zed Business

## Concerns and Questions

If you have concerns about telemetry, you can [open an issue](https://github.com/zed-industries/zed/issues/new/choose) or email hi@zed.dev.
