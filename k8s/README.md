# Kubernetes deployment

k3s. The ingress assumes traefik and cert-manager, and the volume assumes the `local-path`
provisioner. One pod: the emulator, the agent and the web server are one process, and a run's whole
state is one directory on one volume.

`secret.example.yml` lives outside `gb/` on purpose. Everything in `gb/` is applied as a directory,
so a Secret template full of placeholders in there would blank the real API key and admin token on
every re-apply. Copy it to `gb/secret.axh.yml` (gitignored), fill it in, and it is applied with the
rest.

Set `GB_POLICY` and `GB_MODEL` in `gb/configmap.yml` and the hostname in `ingress.yml`, then apply
`redirect-http-https.yml`, `./gb` and `ingress.yml` in that order. The namespace must be `gb`: the
ingress names the redirect middleware as `gb-redirect-http-https@kubernetescrd`, and traefik resolves
that by namespace.


## The run

The binary resumes by default: it continues the newest run under `/runs` whose save state loads, so a
rollout, a deleted pod or a node reboot all pick the playthrough up where it was. The 30 s grace
period is deliberate — SIGTERM is what writes that checkpoint.

With `GB_ADMIN_TOKEN` set, `/reset-game` starts the game over from a browser and
`POST /api/new-run` does it from a script; `POST /api/clear` keeps the run and throws away only what
the model remembers of it, which is the one to reach for when a run is walking into the same wall and
filing bug reports about it. All three 404 while the token is unset *or blank*, which is the shape a
placeholder Secret takes, and a clear lands on the model's next turn — so a run parked on a spent
quota is cleared when the quota reopens.

The volume grows on its own: a run that finishes the game is archived to `/runs/hall-of-fame/` and
the next run starts automatically. An archive is a few hundred kilobytes against the PVC's 1 Gi, but
it is unbounded in the number of wins and nothing prunes it.

## What plays the game

`GB_POLICY` in `gb/configmap.yml` — `llm`, `random` or `deterministic`. The `--policy` flag still
exists and still wins, which makes it the override for a one-off rather than the configuration.

A ConfigMap edit restarts nothing: `envFrom` is read once at process start and the pod template has
no checksum annotation, so it is always apply *then* `rollout restart`. Moving to or from `--policy`
on the args is one rollout and the ConfigMap has to land first, or the pod that rolls reads the old
one.

`random` and `deterministic` need no API key and spend nothing, and both exercise the whole stack.
`deterministic` starts in Red's bedroom and every step is relative to that, so it wants a game at the
beginning: apply, restart, then `POST /api/new-run`. Resumed onto a mid-game save it replays a route
the world has already moved past.

## The image

`ghcr.io/axle-h/gb:latest`, published on every push to main *after* a smoke test that proves the
image serves the SPA, decodes the cartridge and emulates. Every build is also tagged with its commit,
which is the tag to pin or roll back to. `imagePullPolicy: Always` means a `rollout restart` picks up
the newest build; nothing watches the registry, because a rollout interrupts a live playthrough.

GHCR creates a package private regardless of the repository's visibility, so the first CI push
succeeds and the cluster then fails to pull with `denied`. Make it public once, in the package's own
settings.
