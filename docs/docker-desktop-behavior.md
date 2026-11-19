# Docker Desktop behavior

The reference build is Docker Desktop 4.99.0 on macOS arm64 with Engine 29.9.3. REA listed the files in the Electron archive and checked ten dashboard modules. The retained analysis is `ev_efa6a9a56068071d36df4c91cc7c2d67293d8f2dcf34ce32a076a7e028fe2302`. This covers part of the app. Four unpacked helpers failed integrity checks, so the spec leaves them out. REA could not capture the live dashboard. The first attempt timed out, and the retry failed because REA could not verify the process identity.

| Surface | User-visible behavior |
| --- | --- |
| Containers | Find containers and Compose projects, narrow the list to running containers, and use state-dependent lifecycle actions. Details expose logs, configuration, mounts, debugging, files, and resource usage. Published ports can open in a browser. [Containers documentation](https://docs.docker.com/desktop/use-desktop/container/) |
| Images and volumes | Browse tags, pull or run images, and view image details. Volumes show usage and files. You can import and export volume data. [Images documentation](https://docs.docker.com/desktop/use-desktop/images/) [Volumes documentation](https://docs.docker.com/desktop/use-desktop/volumes/) |
| Builds and logs | Inspect active builds and BuildKit history with logs and traces. A separate Logs view combines container and build output with search, filters, presets, and export. [Builds documentation](https://docs.docker.com/desktop/use-desktop/builds/) [Logs documentation](https://docs.docker.com/desktop/use-desktop/logs/) |
| Kubernetes | Provision a local cluster, then browse live pods, services, and deployments by namespace. [Kubernetes documentation](https://docs.docker.com/desktop/use-desktop/kubernetes/) |
| Product and engine controls | Quick search jumps to resources. Settings configure virtualization, resource limits, file sharing, networking, and builders. The dashboard also exposes sign-in, diagnostics, updates, extensions, Model Runner, Gordon, MCP Toolkit, and cloud features subject to availability. [Dashboard documentation](https://docs.docker.com/desktop/use-desktop/) [Settings documentation](https://docs.docker.com/desktop/settings-and-maintenance/settings/) |

## Ferrum lifecycle behavior

Both SSH container views offer Pause when a Docker or Podman container is running. A paused container offers Resume, which runs `unpause`. A stopped container offers Start when the runtime can start it. In the manager, you can filter by running, paused, or stopped and still search by text. The list groups containers by runtime and Compose project, and pods by namespace.

Use the active SSH session and a validated container ID for each action. Report the result and wait for a fresh list before enabling the next action. Keep Ferrum's visual design and use the same action rules in both views and the SSH command path.

### Reference inputs

Three test containers used the local `redis:7` image. Each ran `sleep 600` with its network off. The bundled CLI ran these actions on the Desktop engine.

| Input state | Action | Engine state after action |
| --- | --- | --- |
| `running` | Pause | `paused` |
| `paused` | Resume | `running` |
| `exited` | Start | `running` |

Ferrum's SSH test replays the three rows from the original engine and checks the filter, available actions, and commands sent over SSH.

## Ferrum images and volumes

The Images and Volumes views list Docker resources on the active SSH host. Search by image tag or ID, or by volume name or driver. Selecting a row shows metadata. Image details include tags, digests, platform and layers. Volume details include driver, scope, creation time and the engine's mount path.

Pull an image by reference or create a named volume on that host. The view reports the result and reloads the list. Ferrum selects which metadata the remote CLI returns, so image environment variables and volume driver options stay on the host.

REA analyzed six shipped image and volume modules in `ev_b821c55cf3f5a297ea81bd5c82f8f39a5754d93c0d9f4d0c20aa9c3efafe8b15`. The bundled CLI on Engine 29.9.3 supplied three reference cases. It listed a tagged image, inspected its platform and layers, and created, listed, and inspected a disposable named volume.

## Ferrum build history

The Builds tab reads the configured Buildx builder on the active SSH host. Search by build name, status, or reference. A selected build shows its status, step counts, and available log. The log can be searched by line and copied. Refresh reads the current history again.

REA analyzed five shipped Builds modules in `ev_e2a2895a3a7f4e32bae508f2917ee45a6417913cbe0e8b8244b37e0e17b4cb03`. The reference CLI was Buildx 0.38.1. The engine returned a completed scratch build, a failed scratch build, and the failed build's missing-file log. Buildx listed each record as `builder/node/id`. Log retrieval accepted `--builder builder` and the final ID component. [Build history list](https://docs.docker.com/reference/cli/docker/buildx/history/ls/) [Build history logs](https://docs.docker.com/reference/cli/docker/buildx/history/logs/)

## Ferrum unified logs

The Unified logs tab reads snapshots from up to eight selected Docker or Podman containers, Kubernetes pods, and Buildx records on the active SSH host. It keeps up to 300 lines per source. The table orders timestamped output by time and identifies the source of each line. Refresh on demand or enable five-second updates while the tab is open. Search by source or message, select a row to read the full line, or copy the matching lines. Errors appear beside the available output from other sources.

REA analyzed four shipped log modules in `ev_c70aa00e3784b4b2848147d859b3d01afac71da153034886eb0097636251ea87`. The reference engine supplied a container stdout line, a container stderr line, and a timestamped Buildx failure from a scratch build. Container logs prefix RFC 3339 timestamps. Buildx raw JSON carries timestamps on log chunks and completed failed steps. [Docker Desktop Logs](https://docs.docker.com/desktop/use-desktop/logs/) [Docker container logs](https://docs.docker.com/reference/cli/docker/container/logs/)

### Search, saved filters, and export

Search accepts literal text or a regular expression, with optional case matching. An invalid expression shows an error and blocks copying or exporting matches. A saved filter stores the query, search options, and selected sources for one saved SSH host in the encrypted vault. Applying it reloads the logs if the source set changes.

Export all loaded rows or only matches to a local JSON Lines file. Each record includes the UTC timestamp, source, and full message. The Docker Desktop engine supplied three reference messages for the search and export tests. They cover an ordinary status line, a warning, and an error. [Docker Desktop Logs](https://docs.docker.com/desktop/use-desktop/logs/)

## Ferrum volume files and export

Open a standard local volume and choose **Browse files**. The browser lists folders, file sizes, and symbolic links. It preserves the original filename bytes during navigation. **Export volume** saves the whole volume as a tar archive over SSH and publishes the destination file only after the helper exits successfully.

REA analyzed four shipped volume file and transfer modules in `ev_d57a74b766961b4282f836a25893e43f4e7a7892a08f6e0c88f8beba9209c389`. A disposable volume on Docker Desktop Engine 29.9.3 supplied three inputs. Ferrum checked a root listing, a nested listing with a newline in the filename, and an archive containing a binary file and a symbolic link. [Docker Desktop volumes](https://docs.docker.com/desktop/use-desktop/volumes/)

### Import an archive

Enter a fresh volume name in **Volumes** and choose **Import archive**. Ferrum validates a local tar or gzip-compressed tar before contacting Docker. It creates a new standard local volume and streams the archive over SSH. The import preserves file bytes, folders, internal links, and ordinary permissions.

Ferrum checks that the new volume belongs to the import before writing. An existing name is left untouched. If extraction fails, Ferrum attempts to remove the volume it created and reports any cleanup error. [Docker Desktop volume import](https://docs.docker.com/desktop/use-desktop/volumes/#import-a-volume)

### Edit a volume file

Choose **Edit text** in the volume browser to open a UTF-8 file up to 512 KiB. **Save file** stages the draft beside the original, verifies the uploaded bytes, checks the opened version again, then renames the draft into place. Ferrum preserves numeric ownership and Unix mode. A conflict leaves the draft open. Files with multiple hard links open read-only. [Docker Desktop stored data](https://docs.docker.com/desktop/use-desktop/volumes/#stored-data)

REA inspected the shipped volume contents and details modules in `ev_4c14efbcb27540a69bbb44b1047a148e0f73b22907cc0f37be6085abde49a213`. The Ferrum helper ran against three cases on the Docker Desktop engine. It updated a file owned by a non-root user, rejected a stale save, and kept a hard-linked file read-only.
