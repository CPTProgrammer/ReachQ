---
title: Port Tunneling
description: Forward ports through SSH connections.
---

Reach supports SSH port tunneling so you can securely access services behind firewalls or only listening on private networks.

## Tunnel Types

Three types are available in the UI:

- **Local** — binds a port on your machine, forwards traffic through SSH to a remote destination. This is the one you'll use most.
- **Remote** — binds a port on the remote server and forwards back to your machine. Planned but not implemented yet.
- **Dynamic** — SOCKS proxy through your SSH connection. Also planned but not implemented yet.

Local forwarding is the only type that actually works right now. Remote and Dynamic show up in the type selector but will return an error if you try to start them.

## Creating a Tunnel

Open the Tunnels panel in the sidebar (the chain-link icon). Click **New Tunnel**. A form appears with:

- **Type** — three buttons: Local Forward, Remote Forward, Dynamic (SOCKS). Selected type is highlighted in accent color.
- **Listen Port** — the port to listen on (e.g., `8080`). Must be 1-65535. Local and Dynamic tunnels bind it on your machine; Remote tunnels bind it on the server.
- **Target Host** — where the traffic goes on the other end, as seen from the SSH server (e.g., `localhost`, `10.0.0.5`). Can't be empty. Hidden for Dynamic type, which has no fixed target.
- **Target Port** — the port on the target host (e.g., `3306`). Must be 1-65535. Also hidden for Dynamic type.

A live preview at the bottom of the form shows the resulting mapping (e.g. `localhost:8080 → 10.0.0.5:3306`; Dynamic shows `localhost:1080 → SOCKS`).

Hit **Create**. The tunnel gets created but doesn't start automatically — it's inactive by default. You'll see a toast confirming creation with the port number.

## Persistence

Tunnels created while a **saved session** tab is active are stored encrypted in the vault, bound to that session, and survive app restarts. Starting one only requires that *some* connection of that session is live — by default the connection of the tab you're looking at. If the session isn't connected, starting fails with a "session not connected" error.

Tunnels created from a **quick connect** (no saved session) are ephemeral: they live only for the app's lifetime and disappear on exit.

The panel is scoped to the active tab (like the file explorer): session-bound tunnels appear on any tab of their session, quick-connect tunnels only on the tab that created them.

## Tunnel Lifecycle

A tunnel rides one specific SSH connection and **stops when that connection closes** — whether you close the tab, disconnect, or the network drops. The tunnel configuration persists; reconnect the session and start it again. Closing a tab also shuts the SSH transport down for real, so no orphaned background connections linger.

## Starting and Stopping

Each tunnel shows up as a card with a status dot:

- **Gray dot** — inactive
- **Green dot with glow** — active and running

Hover over the card to reveal action buttons:

- **Play button** (green) — starts the tunnel
- **Stop button** (red square) — stops it

When you start a local tunnel, Reach binds a TCP listener on `127.0.0.1:{local_port}`. Any connection to that port gets relayed through an SSH `direct-tcpip` channel to `{remote_host}:{remote_port}` on the other end. Data flows both ways until either side closes.

## Tunnel Cards

Each card shows:

- **Type badge** — a colored letter: **L** (blue) for Local, **R** (orange) for Remote, **D** (green) for Dynamic
- **Mapping** — the forwarding mapping in monospace: `localhost:{listen_port} → {target_host}:{target_port}` for Local, `server:{listen_port} → {target_host}:{target_port}` for Remote, and `localhost:{listen_port} → SOCKS` for Dynamic
- **Status dot** — gray or green

The trash icon on the right deletes the tunnel.

## Typical Use Case

Say there's a MySQL server on `10.0.0.5:3306` that's only reachable from your SSH host. Create a local tunnel:

- Listen Port: `3306`
- Target Host: `10.0.0.5`
- Target Port: `3306`

Start it. Now point your database client at `localhost:3306` and traffic goes through the SSH tunnel. Same thing as `ssh -L 3306:10.0.0.5:3306`, just with a UI.

## How It Works Under the Hood

When you start a local tunnel, the backend:

1. Binds a `TcpListener` on `127.0.0.1:{local_port}`
2. Spawns an async task that accepts incoming connections
3. For each connection, opens an SSH `direct-tcpip` channel to `{remote_host}:{remote_port}`
4. Relays data bidirectionally using `tokio::select!`
5. Handles EOF and errors on both sides gracefully

When you stop the tunnel, it sends a shutdown signal and waits up to 2 seconds for cleanup.

## Plugin Hooks

Plugins get notified about tunnel events:

- `tunnel_started(tunnel_id, local_port)` — fires when a tunnel starts
- `tunnel_stopped(tunnel_id)` — fires when a tunnel stops

## Limitations

- Remote and Dynamic forwarding are defined in the type system but not implemented yet.
- Tunnels don't outlive their connection — see Tunnel Lifecycle above.
- Port conflicts (another process already using the local port) are caught and reported as errors.
