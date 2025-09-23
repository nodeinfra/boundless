#!/usr/bin/env python3
import re
import subprocess
import sys
import time
import os
import datetime

# Regex to capture the order id
ORDER_REGEX = re.compile(r"(0x[a-fA-F0-9]{64})")

# Track already reset orders with timestamp to avoid repeated resets
reset_orders: dict[str, float] = {}
RESET_COOLDOWN_SEC = 300  # don't reset same order more than once every 5 minutes

def reset_order(order_id: str, original_line: str):
    last_reset = reset_orders.get(order_id)
    now = time.time()
    if last_reset and (now - last_reset) < RESET_COOLDOWN_SEC:
        print(f"[auto-reset] Skipping reset for {order_id}, last reset {int(now - last_reset)}s ago", flush=True)
        return

    print(f"[auto-reset] Detected failed proof for order {order_id}, resetting...", flush=True)

    # Save debugging info into logfile
    logdir = "auto-reset-logs"
    os.makedirs(logdir, exist_ok=True)
    timestamp = datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ")
    logfile = os.path.join(logdir, f"{order_id}_{timestamp}.log")
    with open(logfile, "w") as f:
        f.write(f"Order ID: {order_id}\n")
        f.write(f"Triggered at: {timestamp} UTC\n")
        f.write("Original log line:\n")
        f.write(original_line + "\n\n")

        # Capture running containers
        try:
            ps_out = subprocess.check_output(["docker", "ps"], text=True)
            f.write("=== docker ps ===\n")
            f.write(ps_out + "\n")
        except Exception as e:
            f.write(f"Failed to run docker ps: {e}\n")

        # Capture logs for selected containers (last 3 min)
        containers = [
            "bento-broker-1",
            "bento-rest_api-1",
            "bento-gpu_prove_agent0-1",
            "bento-aux_agent-1",
        ] + [f"bento-exec_agent{i}-1" for i in range(0,5)]
        for c in containers:
            try:
                logs_out = subprocess.check_output(
                    ["docker", "logs", "--since=3m", c],
                    text=True, stderr=subprocess.STDOUT
                )
                f.write(f"\n=== docker logs --since=3m {c} ===\n")
                f.write(logs_out + "\n")
            except Exception as e:
                f.write(f"Failed to get logs for {c}: {e}\n")

    try:
        subprocess.run(
            ["./scripts/reset-order.sh", order_id],
            check=True
        )
        print(f"[auto-reset] Reset executed successfully for order {order_id}", flush=True)
        reset_orders[order_id] = now
    except subprocess.CalledProcessError as e:
        print(f"[auto-reset] Failed to reset order {order_id}: {e}", file=sys.stderr, flush=True)

def main():
    for line in sys.stdin:
        if (
            "Proving failed after retries" in line
            and "Monitoring proof (stark) failed: [B-BON-005] Prover failure: SessionId" in line
        ):
            match = ORDER_REGEX.search(line)
            if match:
                order_id = match.group(1)
                reset_order(order_id, line.strip())

if __name__ == "__main__":
    main()