# Kubernetes Deployment

I use k3s. This might not work otherwise — the ingress assumes traefik and cert-manager, and the
volume assumes the `local-path` provisioner.

One pod: the emulator, the agent and the web server are one process (`src/host.rs`), and a run's
whole state is one directory on one volume. There is nothing to scale and nothing to shard.

⚠️ **`secret.example.yml` lives outside `gb/` on purpose, and that is not tidying.** Everything in
`gb/` is applied as a directory, and a Secret template full of placeholders sitting in there means
`kubectl apply -f ./gb` blanks the real API key and the real `GB_ADMIN_TOKEN` every time anyone
re-applies the deployment. So the template is kept where a directory apply cannot reach it: copy it
to `gb/secret.axh.yml` (gitignored), fill it in, and it is applied along with everything else.

Set `GB_POLICY` and `GB_MODEL` in `gb/configmap.yml` and the hostname in `ingress.yml` first.

```shell
# Create the namespace. ⚠️ It must be `gb`: the ingress names the redirect middleware as
# `gb-redirect-http-https@kubernetescrd`, and traefik resolves that by namespace.
kubectl create namespace gb

# Create the redirect middleware
kubectl -n gb apply -f ./redirect-http-https.yml

# Your secrets, where a directory apply will keep them rather than overwrite them.
cp ./secret.example.yml ./gb/secret.axh.yml && $EDITOR ./gb/secret.axh.yml

# Create the volume, config, secret and deployment
kubectl -n gb apply -f ./gb

# Check it is UP — the pod is Running and the PVC is Bound
kubectl -n gb get all,pvc

# Create ingress
kubectl -n gb apply -f ./ingress.yml
```

## The run

`gb serve` **resumes** by default: it continues the newest run under `/runs` whose save state loads,
notes and all. So a rollout, a `kubectl delete pod`, a node reboot — all of them pick the
playthrough up where it was, and the pod is deliberately given a 30 s grace period because SIGTERM
is what writes that checkpoint.

To start the game over, set `GB_ADMIN_TOKEN` in the Secret and open
<https://gb.ax-h.com/reset-game>. The browser asks for a password — any user name, the token as the
password — and the game restarts. Or, from a script:

```shell
curl -X POST -H "X-GB-Token: $GB_ADMIN_TOKEN" https://gb.ax-h.com/api/new-run
# → {"run_id":"run-20260811-142233"}
```

No rollout, no downtime, nothing to remember to undo — the current run is checkpointed and left
complete on the volume, and the page follows the new one on its own. ⚠️ Both 404 while
`GB_ADMIN_TOKEN` is unset *or blank*, which is the shape a placeholder Secret takes.

⚠️ **The volume now grows on its own, because a run that finishes the game files itself.** The win is
archived to `/runs/hall-of-fame/<date>-<run-id>/` — save state, SRAM, the model's notes and the whole
transcript, gzipped — and the next run starts automatically. The transcript is the big part and
compresses ten- to twentyfold, so an archive is typically a few hundred kilobytes against the PVC's
1 Gi; but it is unbounded in the number of wins, and nothing prunes it. `kubectl -n gb exec deploy/gb
-- du -sh /runs/hall-of-fame` is the thing to look at if the volume ever fills.

The old way still works and is the fallback if the process is not answering: uncomment the `args:`
line in `gb/deployment.yml` for one rollout and then take it back out. ⚠️ Left in place, every
restart would wipe the run — which is the reason the endpoint exists.

```shell
kubectl -n gb logs -f deploy/gb
kubectl -n gb rollout restart deploy/gb    # resumes where it was
kubectl -n gb exec deploy/gb -- ls /runs   # the run directories
```

## What plays the game

`GB_POLICY` in `gb/configmap.yml` — `llm`, `random` or `deterministic`. It used to be `--policy` on
the container's `args`; the flag still exists and still wins over the variable, which makes it the
override for a one-off `kubectl -n gb set args` rather than the standing configuration.

⚠️ **A ConfigMap edit restarts nothing** — `envFrom` is read once at process start and the pod
template has no checksum annotation, so it is always apply *then* restart:

```shell
$EDITOR ./gb/configmap.yml
kubectl -n gb apply -f ./gb/configmap.yml
kubectl -n gb rollout restart deploy/gb
```

Two of the three need no API key, no model and no spend, and both exercise the whole stack — the
agent, the video pipeline, the page and the volume:

- **`random`** picks a legal action at random. This is the one to switch to while an agent bug is
  being fixed or the credit has run out, rather than leaving the pod erroring at every turn.
- **`deterministic`** plays the scripted route `full_playthrough` plays, at 1× on the page instead of
  in a test harness — a whole playthrough to watch, for nothing. ⚠️ **It starts in Red's bedroom and
  every step is relative to that**, so it wants a game at the beginning: apply, restart, and then
  `POST /api/new-run`. Resumed onto a mid-game save it replays a route the world has already moved
  past. ⚠️ **It ends on Victory Road 2F rather than in the Hall of Fame**, because that step list
  does: the VR2F/VR3F puzzle and the Elite Four are left out of it as PP-marginal for the team the
  route arrives with (they are proved separately, from their own fixtures). The run parks there and
  files no hall-of-fame entry.

⚠️ **Moving to or from `--policy` on the args is one rollout, and the ConfigMap has to land first.**
Applying `deployment.yml` changes the args and therefore rolls the pod, which is what picks up the
variable; applying it before the ConfigMap rolls a pod that reads the old one.

## The image

`ghcr.io/axle-h/gb:latest`, built from the repo root `Dockerfile` — four stages, because two of the
crate's compile-time inputs are generated and neither is in git: the cartridge (`pokered.gbc`,
assembled by rgbds and checked against upstream's sha1) and the SPA (`web/dist`, baked in by
`rust-embed`).

The `container` workflow publishes it on every push to `main`, **after** the smoke test that proves
the image serves the SPA, decodes the cartridge and emulates — so `:latest` is always an image that
was seen to run. Every build is also tagged with its commit, which is the tag to pin or roll back to:

```shell
kubectl -n gb set image deploy/gb gb=ghcr.io/axle-h/gb:<sha>
```

`imagePullPolicy: Always` on `:latest` means a `kubectl rollout restart` picks up the newest build.
Nothing here watches the registry — there is no auto-deploy, by choice: a rollout interrupts a live
playthrough (it resumes from the last checkpoint, but it does interrupt).

⚠️ **The package is private until it is made public, once.** GHCR creates it private regardless of
the repository's visibility, so the first CI push succeeds and the cluster then fails to pull with
`denied`. Fix it at
<https://github.com/users/axle-h/packages/container/gb/settings> → *Danger Zone* → *Change
visibility* → **Public**. A private package is workable too, but then the namespace needs an
`imagePullSecret` that this directory does not have.
