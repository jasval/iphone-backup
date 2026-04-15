# AdGuard Home – DNS Rewrite

Add a rewrite so `iphone-backup.home` resolves to your NAS LAN IP:

1. AdGuard Home → Filters → DNS Rewrites → Add DNS Rewrite
2. Domain:  iphone-backup.home
   Answer:  <your-NAS-LAN-IP>  (e.g. 192.168.1.100)

For Tailscale, MagicDNS makes the NAS automatically available at:
  https://<nas-hostname>.<your-tailnet>.ts.net
No extra DNS config needed for Tailscale.
