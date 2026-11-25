# Optional GitHub connection

CVEC discovers local Git repositories under the selected folder. You can clone a private repository with your usual Git credentials and place it under that folder. Public GitHub URLs also work anonymously. Configure your own GitHub App for sign-in, authenticated API limits and direct access to selected private repositories. Requests go directly from CVEC to GitHub. See [first-run setup](getting-started.md#first-run) for local discovery.

For a linked repository, CVEC downloads the default branch's exact commit into a temporary scan directory and verifies file contents against the commit's Git objects. Coverage details identify unavailable submodule contents, Git LFS objects, and unresolved symbolic links. A local checkout can supply those files and resolved dependency metadata such as NuGet lockfiles.

## Register and install your GitHub App

1. In GitHub **Settings → Developer settings → GitHub Apps**, register an app under your account or organization. Give it a name and a homepage URL, such as the owning account's profile. Enable **Device Flow**. Leave **Request user authorization (OAuth) during installation** and **Webhook → Active** off. [GitHub registration guide](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)
2. Grant repository **Contents: Read-only**. GitHub includes **Metadata: Read-only**. Choose **Only on this account** for an app used by its owner.
3. Use **Install App** and select the repositories CVEC should access. For an organization, its policy may require approval. The installation and your GitHub account both need access to a private repository. [GitHub installation guide](https://docs.github.com/en/apps/using-github-apps/installing-a-github-app-from-a-third-party)

## Configure CVEC and sign in

1. In **Settings → General → GitHub account**, select **Set up GitHub App**. Paste the public **Client ID** from your GitHub App's settings and select **Save app**. Add the optional GitHub installation link to enable **Manage app**.
2. Select **Use saved sign-in** if you already signed in with this Client ID on the same Mac. macOS may prompt for Keychain access. Otherwise, select **Sign in**, enter the code displayed by CVEC on GitHub, and authorize your app. [Local development and Keychain](local-development-keychain.md) · [GitHub device flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token)
3. Select **Load repos** and choose an installed repository to import and scan it. On later launches, use **Use saved sign-in** to reconnect.

CVEC stores the Client ID and installation link in local preferences. Access tokens are stored in the macOS Keychain. **Edit GitHub App** changes the preferences. **Sign out** removes the saved sign-in, and **Remove app** clears the preferences and disables the connection. GitHub manages installation and authorization.

Launch variables `CVEC_GITHUB_APP_CLIENT_ID` and `CVEC_GITHUB_APP_INSTALL_URL` provide the initial configuration for existing command-line setups. Settings saved in CVEC take precedence.
