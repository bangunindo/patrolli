---
description: Sign in to every Patrolli cluster with one Keycloak account and switch clusters without signing in again.
---

# Authentication

This guide adds single sign-on with Keycloak to the deployment from [Multiple Cluster](./multiple-cluster). After you sign in to Keycloak once, the web interface shows data from every cluster where you have an account, and you switch clusters from the top bar.

Keycloak checks the password, but it does not create users in Patrolli. Each cluster keeps its own users. A Keycloak login works on a cluster only when that cluster has a user with the same username and with **Allow SSO** turned on. The first `admin` user of each cluster already allows it, because its `compose.yaml` sets `PATROLLI_ADMIN_ALLOW_SSO: "true"`.

## Before you begin

You need the following:

- The deployment from [Multiple Cluster](./multiple-cluster), running. Run every command of this guide in its `patrolli` directory.
- `openssl` on the host.
- The IP address of the host. This guide uses `192.168.1.10` in commands, files, and Keycloak settings. Replace it everywhere with the IP address of your host. The browser must reach this address on port `3000` and port `8443`.

## 1. Create a certificate for Keycloak

In this guide, Keycloak uses HTTPS. The commands below create a small certificate authority (CA) and use it to sign a certificate for the IP address of the host. Later steps tell the web interface and the clusters to trust this CA.

```sh
mkdir certs keycloak
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -keyout certs/ca.key -out certs/ca.crt -subj "/CN=Patrolli test CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign"
printf "subjectAltName=IP:192.168.1.10\n" > certs/keycloak.ext
openssl req -newkey rsa:2048 -nodes \
    -keyout certs/keycloak.key -out certs/keycloak.csr -subj "/CN=192.168.1.10"
openssl x509 -req -in certs/keycloak.csr -CA certs/ca.crt -CAkey certs/ca.key \
    -days 825 -extfile certs/keycloak.ext -out certs/keycloak.crt
chmod 644 certs/keycloak.key
```

The Keycloak container runs as user ID `1000`, so `chmod` lets it read `keycloak.key`. Keep `ca.key` private. Anyone with this file can sign certificates that your clusters trust.

Check the new certificate:

```sh
openssl verify -CAfile certs/ca.crt certs/keycloak.crt
```

The command prints `certs/keycloak.crt: OK`.

If your organization already has a CA, use a certificate from it instead. Put that CA in `certs/ca.crt` and the Keycloak certificate and key in `certs/keycloak.crt` and `certs/keycloak.key`.

## 2. Start Keycloak

Create the Compose file for Keycloak:

::: code-group

```yaml [keycloak/compose.yaml]
name: patrolli-keycloak

services:
    patrolli-keycloak:
        image: quay.io/keycloak/keycloak:26.8.1
        container_name: patrolli-keycloak
        restart: unless-stopped
        command: start-dev
        environment:
            KC_BOOTSTRAP_ADMIN_USERNAME: keycloak-admin
            KC_BOOTSTRAP_ADMIN_PASSWORD: change-me
            KC_HOSTNAME: https://192.168.1.10:8443
            KC_HTTPS_CERTIFICATE_FILE: /opt/keycloak/conf/keycloak.crt
            KC_HTTPS_CERTIFICATE_KEY_FILE: /opt/keycloak/conf/keycloak.key
        ports:
            - "8443:8443"
        volumes:
            - ../certs/keycloak.crt:/opt/keycloak/conf/keycloak.crt:ro
            - ../certs/keycloak.key:/opt/keycloak/conf/keycloak.key:ro
            - data:/opt/keycloak/data
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

:::

- **`KC_BOOTSTRAP_ADMIN_USERNAME` and `KC_BOOTSTRAP_ADMIN_PASSWORD`:** the account for the Keycloak admin console. It is not a Patrolli user.
- **`KC_HOSTNAME`:** the address that Keycloak writes into its tokens and links. The browser, the web interface, and the clusters all use this address.

Start Keycloak:

```sh
docker compose -f keycloak/compose.yaml up -d
```

Wait until the log shows that Keycloak started:

```sh
docker logs patrolli-keycloak 2>&1 | grep "started in"
```

```text
2026-10-22 10:23:20,865 INFO  [io.quarkus] (main) Keycloak 26.8.1 on JVM (powered by Quarkus 3.40.1) started in 14.708s. Listening on: http://0.0.0.0:8080 and https://0.0.0.0:8443
```

::: info
`start-dev` runs Keycloak in development mode, which is enough for a test. For production, follow the [Keycloak server guides](https://www.keycloak.org/guides#server).
:::

## 3. Set up Keycloak

Open `https://192.168.1.10:8443/admin/` and sign in with `keycloak-admin` and `change-me`. The browser warns that it does not trust the certificate, because your own CA signed it. Accept the warning for this test, or import `certs/ca.crt` into the trusted certificates of your computer.

1. **Create a realm**

    A realm holds the users and the clients of one organization. Click **Manage realms**, then **Create realm**. Enter `patrolli` as the **Realm name**, and click **Create**.

    ![The Create realm dialog with patrolli as the realm name](./assets/authentication/create-realm.png)

    Keycloak switches to the new realm. Check that the left menu shows **patrolli** as the current realm before you go on. The next steps must happen in this realm, not in `master`.

    ![The left menu shows patrolli as the current realm](./assets/authentication/current-realm.png)

2. **Create a client for the web interface**

    Click **Clients**, then **Create client**. Enter `patrolli-web` as the **Client ID**. The **Name** is optional, and this guide uses `Patrolli web interface`. Click **Next**.

    ![General settings with patrolli-web as the client ID](./assets/authentication/client-general.png)

    Turn on **Client authentication**. Under **Authentication flow**, keep only **Standard flow** selected. Click **Next**.

    ![Capability config with Client authentication on and only Standard flow selected](./assets/authentication/client-capability.png)

    Fill in the addresses of the web interface, and click **Save**:

    - **Root URL:** `http://192.168.1.10:3000`
    - **Valid redirect URIs:** `http://192.168.1.10:3000/auth/oidc/callback`
    - **Web origins:** `http://192.168.1.10:3000`

    ![Login settings with the three addresses of the web interface](./assets/authentication/client-login-settings.png)

3. **Copy the client secret**

    Open the **Credentials** tab of the client and copy the **Client Secret**. The web interface needs it in the next section.

    ![The Credentials tab with the copy button next to the client secret](./assets/authentication/client-secret.png)

4. **Add the client ID to the access token**

    The web interface sends the Keycloak access token to the clusters, and each cluster accepts the token only when the audience of the token (`aud`) contains `patrolli-web`. Keycloak does not add the client ID to access tokens by itself, so add a mapper for it.

    Open the **Client scopes** tab of the client and click **patrolli-web-dedicated**. Click **Configure a new mapper**, and choose **Audience**. Enter a **Name**, such as `patrolli-web audience`. Under **Included Client Audience**, click **Select client**, pick `patrolli-web`, and click **Add**.

    ![The Select client dialog with patrolli-web selected](./assets/authentication/audience-select-client.png)

    Keep **Add to access token** on, and click **Save**.

    ![The Audience mapper with patrolli-web as the included client audience](./assets/authentication/audience-mapper.png)

5. **Create a user**

    Click **Users**, then **Create new user**. When the realm already has users, the button is **Add user**. Turn on **Email verified**, and enter `admin` as the **Username**. Fill in **Email**, **First name**, and **Last name** too, or Keycloak asks for them at the first sign-in. Click **Create**.

    ![The Create user form with admin as the username](./assets/authentication/create-user.png)

    The username must match a Patrolli user. Here it is the `admin` user that each cluster created at its first start.

6. **Set a password**

    Open the **Credentials** tab of the user and click **Set password**. Enter the password twice, turn off **Temporary**, and click **Save**. Confirm with **Save password**. This guide uses `change-me`.

    ![The Set password dialog with Temporary turned off](./assets/authentication/set-password.png)

## 4. Connect the web interface

Mount the CA in the `patrolli-web` container, and add the `auth` block to `config-fe.yaml`. The highlighted lines are new. Replace `REPLACE_WITH_CLIENT_SECRET` with the client secret that you copied.

::: code-group

```yaml{12} [web/compose.yaml]
name: patrolli-web

services:
    patrolli-web:
        image: patrolli-web
        container_name: patrolli-web
        restart: unless-stopped
        ports:
            - "3000:3000"
        volumes:
            - ./config-fe.yaml:/app/config-fe.yaml:ro
            - ../certs/ca.crt:/certs/ca.crt:ro
        networks:
            - patrolli

networks:
    patrolli:
        external: true
```

```yaml{9-15} [web/config-fe.yaml]
clusters:
    - name: Cluster A
      url: http://patrolli-a-1:8080
    - name: Cluster B
      url: http://patrolli-b-1:8080
    - name: Cluster C
      url: http://patrolli-c-1:8080

auth:
    keycloak:
        host: https://192.168.1.10:8443/realms/patrolli
        client_id: patrolli-web
        client_secret: REPLACE_WITH_CLIENT_SECRET
        redirect_uri: http://192.168.1.10:3000/auth/oidc/callback
        ca_cert_path: /certs/ca.crt
```

:::

- **`host`:** the address of the realm. Both the browser and the `patrolli-web` container must reach it.
- **`client_id` and `client_secret`:** the client from the previous section.
- **`redirect_uri`:** the address that Keycloak sends the browser back to. The web interface builds this address from the address in the browser and uses `redirect_uri` only when it cannot. Open the web interface with the same address that you listed in **Valid redirect URIs**.
- **`ca_cert_path`:** the CA that signed the Keycloak certificate.

Recreate the container with the new settings:

```sh
docker compose -f web/compose.yaml up -d
```

## 5. Connect the clusters

Each cluster checks the Keycloak tokens by itself. Mount the CA in every node, and add the `oidc` block to every `config.yaml`. The highlighted lines are new.

::: code-group

```yaml{18} [cluster-a/compose.yaml]
name: patrolli-cluster-a

services:
    patrolli-a-1:
        image: patrolli-core
        container_name: patrolli-a-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
            PATROLLI_JWT_SECRET: ${JWT_SECRET:?}
            PATROLLI_SECRET_KEYS: ${SECRET_KEYS:?}
            PATROLLI_SECRET_ACTIVE_KEY: ${SECRET_ACTIVE_KEY:?}
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
            - ../certs/ca.crt:/certs/ca.crt:ro
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

```yaml{18} [cluster-b/compose.yaml]
name: patrolli-cluster-b

services:
    patrolli-b-1:
        image: patrolli-core
        container_name: patrolli-b-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
            PATROLLI_JWT_SECRET: ${JWT_SECRET:?}
            PATROLLI_SECRET_KEYS: ${SECRET_KEYS:?}
            PATROLLI_SECRET_ACTIVE_KEY: ${SECRET_ACTIVE_KEY:?}
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
            - ../certs/ca.crt:/certs/ca.crt:ro
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

```yaml{18} [cluster-c/compose.yaml]
name: patrolli-cluster-c

services:
    patrolli-c-1:
        image: patrolli-core
        container_name: patrolli-c-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
            PATROLLI_JWT_SECRET: ${JWT_SECRET:?}
            PATROLLI_SECRET_KEYS: ${SECRET_KEYS:?}
            PATROLLI_SECRET_ACTIVE_KEY: ${SECRET_ACTIVE_KEY:?}
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
            - ../certs/ca.crt:/certs/ca.crt:ro
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

:::

::: code-group

```yaml{13-19} [cluster-a/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_a
        cluster:
            node_id: 01a162d5-72de-7b62-bc3c-848dd2ea0567
            node_name: node-1
            metadata:
                advertise_address: patrolli-a-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-a
                oidc:
                    issuer: https://192.168.1.10:8443/realms/patrolli
                    jwks_url: https://192.168.1.10:8443/realms/patrolli/protocol/openid-connect/certs
                    audience: patrolli-web
                    username_claim_path: preferred_username
                    tls:
                        ca_cert_path: /certs/ca.crt
```

```yaml{13-19} [cluster-b/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_b
        cluster:
            node_id: 01a162d5-72e1-7788-8aee-af7caa86f1bd
            node_name: node-1
            metadata:
                advertise_address: patrolli-b-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-b
                oidc:
                    issuer: https://192.168.1.10:8443/realms/patrolli
                    jwks_url: https://192.168.1.10:8443/realms/patrolli/protocol/openid-connect/certs
                    audience: patrolli-web
                    username_claim_path: preferred_username
                    tls:
                        ca_cert_path: /certs/ca.crt
```

```yaml{13-19} [cluster-c/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_c
        cluster:
            node_id: 01a162d5-72e3-7d54-9d20-1306d2c9189e
            node_name: node-1
            metadata:
                advertise_address: patrolli-c-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-c
                oidc:
                    issuer: https://192.168.1.10:8443/realms/patrolli
                    jwks_url: https://192.168.1.10:8443/realms/patrolli/protocol/openid-connect/certs
                    audience: patrolli-web
                    username_claim_path: preferred_username
                    tls:
                        ca_cert_path: /certs/ca.crt
```

:::

- **`issuer`:** the realm address that Keycloak writes into every token, which is `KC_HOSTNAME` followed by `/realms/patrolli`. It is the same address as `host` in `config-fe.yaml`.
- **`jwks_url`:** where the node downloads the public keys of the realm. Keycloak lists both addresses, as `issuer` and `jwks_uri`, at `https://192.168.1.10:8443/realms/patrolli/.well-known/openid-configuration`.
- **`audience`:** the client ID. The audience mapper from the previous section puts it into every access token.
- **`username_claim_path`:** the token claim that the node compares with Patrolli usernames. `preferred_username` is the Keycloak username.
- **`tls.ca_cert_path`:** the CA that signed the Keycloak certificate.

Recreate the nodes with the new settings:

```sh
docker compose -f cluster-a/compose.yaml up -d
docker compose -f cluster-b/compose.yaml up -d
docker compose -f cluster-c/compose.yaml up -d
```

Check that each node accepts tokens from the realm. If the command prints nothing, wait a few seconds and run it again:

```sh
docker logs patrolli-a-1 2>&1 | grep "OIDC issuer"
```

```text
2026-10-22T09:20:42.066810788+00:00  INFO patrolli::api::jwt: accepting tokens from OIDC issuer issuer=https://192.168.1.10:8443/realms/patrolli
```

Run the same command for `patrolli-b-1` and `patrolli-c-1`.

## 6. Sign in with Keycloak

1. **Open the sign-in page**

    Open `http://192.168.1.10:3000` and click **Sign in with Keycloak**.

    ![The Patrolli sign-in page with the Sign in with Keycloak button](./assets/authentication/sign-in-with-keycloak.png)

2. **Sign in to Keycloak**

    Keycloak shows the sign-in page of the `patrolli` realm. Enter `admin` and the password from the previous section, and click **Sign In**.

    ![The Keycloak sign-in page of the patrolli realm](./assets/authentication/keycloak-sign-in.png)

3. **See all clusters at once**

    Keycloak sends you back to Patrolli. The box at the top right shows **Select Cluster**, and the pages show data from every cluster where you have an account. Click **Cluster Monitoring**. The page lists the nodes of all three clusters.

    ![Cluster Monitoring lists the nodes of Cluster A, Cluster B, and Cluster C](./assets/authentication/all-clusters.png)

4. **Switch clusters**

    Click the box at the top right and pick a cluster. The pages then show only that cluster. You can pick another cluster at any time without signing in again.

    ![The cluster list in the top bar shows Cluster A, Cluster B, and Cluster C](./assets/authentication/switch-cluster.png)

::: info
**Log Out** ends the Patrolli session only. While the Keycloak session lasts, **Sign in with Keycloak** signs you in again without a password. To sign in as another user, use a private browser window.
:::

## Add another user

A new user needs an account in Keycloak and an account in every cluster that the user works on. This example adds the user `operator` to Cluster A only.

1. **Create the user in Keycloak**

    Open the admin console and check that the current realm is **patrolli**. If it shows **Keycloak**, the `master` realm, click **Manage realms** and pick **patrolli**. Then repeat steps 5 and 6 of [Set up Keycloak](#_3-set-up-keycloak) with `operator` as the username.

2. **Create the user in a cluster**

    Sign in to Patrolli as `admin` and pick **Cluster A** in the top bar. Click **User Management**, then **Create User**. Enter `operator` as the **Username**, keep **Allow SSO** on, enter a **Password**, and check that **Cluster** shows **Cluster A**. Click **Save**.

    ![The Create User form with operator as the username and Allow SSO on](./assets/authentication/create-patrolli-user.png)

    The username must match the Keycloak username exactly. The form also needs a password, which lets the user sign in to this cluster without Keycloak.

3. **Sign in as the new user**

    In a private browser window, sign in with Keycloak as `operator`. The cluster list shows only **Cluster A**, the one cluster where `operator` has an account. To give the user another cluster, pick that cluster as `admin` and create the user there too.

## Troubleshooting

1. **Keycloak stops at start with `AccessDeniedException /opt/keycloak/conf/keycloak.key`**

    The Keycloak container cannot read the key. Run `chmod 644 certs/keycloak.key` and start Keycloak again.

2. **A node stops at start with `CA certificate file not found`**

    The node cannot find the file in `tls.ca_cert_path`. Check that the `compose.yaml` of the node mounts `../certs/ca.crt` at that path, and run `docker compose -f cluster-a/compose.yaml up -d` again for that cluster.

3. **You are back on the Patrolli sign-in page after you sign in to Keycloak**

    No cluster accepted the Keycloak token, and the web interface shows no message for this. Read the errors that the clusters returned:

    ```sh
    docker logs patrolli-web 2>&1 | grep "Error Detail"
    ```

    - **`Invalid ID token`:** the token does not match the `oidc` block. Check that the client has the audience mapper from step 4 of [Set up Keycloak](#_3-set-up-keycloak) with **Add to access token** on, that `audience` is `patrolli-web`, and that `issuer` is the realm address, character for character.
    - **`Identity provider unreachable`:** the node cannot download the keys from `jwks_url`. Check the address, and check that `tls.ca_cert_path` points to the CA that signed the Keycloak certificate.

4. **A cluster is missing from the cluster list**

    That cluster rejected the token. Read the errors of that cluster, for example Cluster B:

    ```sh
    docker logs patrolli-web 2>&1 | grep "\[Cluster B\]" | grep "Error Detail"
    ```

    - **`No Patrolli account exists for this login`:** the cluster has no user with this username, or the user had **Allow SSO** off at the first sign-in. A Patrolli user also stays linked to the first Keycloak account that signed in with its username. If you delete a Keycloak user and create it again, delete the Patrolli user and create it again too.
    - **`SSO is disabled for this account`:** turn on **Allow SSO** for the user in that cluster.
    - **`This account is disabled`:** set the **Status** of the user in that cluster back to **Active**.
    - **`Invalid ID token` or `Identity provider unreachable`:** the `oidc` block of this cluster differs from the block of a cluster that works. Compare the two.

## Remove the deployment

To delete Keycloak and its data, run:

```sh
docker compose -f keycloak/compose.yaml down -v
```

Then follow [Remove the deployment](./multiple-cluster#remove-the-deployment) in the Multiple Cluster guide to delete the rest.
