// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export function renderOidcSuccessHtml(
	accessToken: string,
	_refreshToken?: string,
	idToken?: string
): string {
	const at = JSON.stringify(accessToken)
	const it = JSON.stringify(idToken || '')

	return `<!DOCTYPE html>
<html lang="en">
<head>
	<meta charset="UTF-8">
	<meta name="viewport" content="width=device-width, initial-scale=1.0">
	<title>Authenticating...</title>
	<style>
		body {
			font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
			display: flex;
			align-items: center;
			justify-content: center;
			min-height: 100vh;
			margin: 0;
			background: #f8fafc;
			color: #1e293b;
		}
		.card {
			text-align: center;
			padding: 2.5rem;
			background: white;
			border-radius: 12px;
			box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
			max-width: 400px;
			width: 90%;
		}
		.spinner {
			width: 40px;
			height: 40px;
			border: 4px solid #e2e8f0;
			border-top-color: #3b82f6;
			border-radius: 50%;
			animation: spin 0.8s linear infinite;
			margin: 0 auto 1.5rem;
		}
		@keyframes spin {
			to { transform: rotate(360deg); }
		}
		h2 { margin: 0 0 0.5rem; font-size: 1.25rem; }
		p { margin: 0; color: #64748b; font-size: 0.875rem; }
	</style>
</head>
<body>
	<div class="card">
		<div class="spinner"></div>
		<h2>Authenticating...</h2>
		<p>Please wait while we redirect you to your dashboard.</p>
	</div>
	<script>
		try {
			var isSecure = window.location.protocol === 'https:';
			var secFlag = isSecure ? '; Secure' : '';
			var at = ${at};
			if (at) {
				document.cookie = "access_token=" + encodeURIComponent(at) + "; path=/; max-age=86400; SameSite=Lax" + secFlag;
			}
			var it = ${it};
			if (it) {
				document.cookie = "id_token=" + encodeURIComponent(it) + "; path=/; max-age=86400; SameSite=Lax" + secFlag;
			}
			document.cookie = "auth_provider=keycloak; path=/; max-age=604800; SameSite=Lax" + secFlag;
			document.cookie = "is_authenticated=true; path=/; max-age=86400; SameSite=Lax" + secFlag;
		} catch (e) {
			console.error('Cookie error:', e);
		}
		window.location.replace('/');
	</script>
</body>
</html>`
}

export function renderOidcErrorHtml(errorMessage: string): string {
	const sanitized = errorMessage
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')

	return `<!DOCTYPE html>
<html lang="en">
<head>
	<meta charset="UTF-8">
	<meta name="viewport" content="width=device-width, initial-scale=1.0">
	<title>Authentication Error</title>
	<style>
		body {
			font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
			display: flex;
			align-items: center;
			justify-content: center;
			min-height: 100vh;
			margin: 0;
			background: #f8fafc;
			color: #1e293b;
		}
		.card {
			text-align: center;
			padding: 2.5rem;
			background: white;
			border-radius: 12px;
			box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
			max-width: 420px;
			width: 90%;
		}
		.icon {
			width: 48px;
			height: 48px;
			background: #fee2e2;
			color: #ef4444;
			border-radius: 50%;
			display: flex;
			align-items: center;
			justify-content: center;
			margin: 0 auto 1.25rem;
			font-size: 1.5rem;
			font-weight: bold;
		}
		h2 { margin: 0 0 0.5rem; font-size: 1.25rem; color: #dc2626; }
		p { margin: 0 0 1.5rem; color: #475569; font-size: 0.875rem; word-break: break-word; }
		a {
			display: block;
			padding: 0.75rem 1.5rem;
			background: #3b82f6;
			color: white;
			text-decoration: none;
			border-radius: 6px;
			font-weight: 500;
			font-size: 0.875rem;
		}
		a:hover { background: #2563eb; }
	</style>
</head>
<body>
	<div class="card">
		<div class="icon">✕</div>
		<h2>Login Failed</h2>
		<p>${sanitized}</p>
		<a href="/account/login">Back to Login</a>
	</div>
</body>
</html>`
}
