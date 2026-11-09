# Microsoft sign-in for Aurora

Aurora signs players in with the official Microsoft device-code flow. Players never type their password into Aurora, and Aurora never stores it.

## How sign-in works

1. Aurora shows a short code and opens the Microsoft sign-in page.
2. The player enters the code and approves Aurora.
3. Aurora exchanges the Microsoft token through Xbox Live and XSTS for a Minecraft token.
4. Aurora checks that the account owns Minecraft Java Edition. Accounts without a Java profile are refused.

## Tokens and how long they last

- **Microsoft refresh token:** stored in Windows Credential Manager under `AuroraLauncher`. It keeps the player signed in across restarts.
- **Minecraft access token:** valid for about 24 hours. Aurora keeps it in memory only, and it refreshes it from the refresh token about five minutes before it expires. Players don't have to sign in again each day.
- **Sign out:** deletes the refresh token and the in-memory token.

Aurora never accepts a token pasted in from another app or another account. Token sharing breaks Mojang's terms and skips the ownership check.

## Before players can sign in: two approvals

Both are required. Without them, sign-in fails at the Minecraft step with an HTTP error, and Aurora says so.

1. **An Azure app registration.** Create it in the Microsoft Entra admin center, as a public client (mobile and desktop). Enable "Allow public client flows". Copy the Application (client) ID.
2. **Mojang approval for the Minecraft services API.** Apply through Mojang's developer channels for access to the Minecraft services API, and give them your client ID. The current application process is in Mojang's developer documentation. I couldn't check it from here, so look it up before you apply.

## Setting the client ID

Open `src-tauri\src\auth.rs` and replace the placeholder:

```rust
const MS_CLIENT_ID: &str = "your-client-id-here";
```

Until this is replaced, Aurora shows "Microsoft sign-in is not set up yet" and doesn't contact Microsoft.

## Testing

Test with an account that owns Minecraft Java Edition, and one that doesn't. The second should show the "does not own Minecraft Java Edition" message. Also check that signing out removes the account, and that a restart signs the player back in without a new code.
