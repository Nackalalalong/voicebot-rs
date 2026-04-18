const API_BASE = '/api/proxy';

export async function apiFetch(path: string, init?: RequestInit) {
    // Strip /api/v1 prefix since proxy adds it
    const cleanPath = path.replace(/^\/api\/v1\//, '');
    const url = `${API_BASE}/${cleanPath}`;

    const res = await fetch(url, {
        ...init,
        headers: {
            'Content-Type': 'application/json',
            ...init?.headers,
        },
    });

    if (!res.ok) {
        const data = await res.json().catch(() => ({error: 'request failed'}));
        throw new Error(data.error ?? `HTTP ${res.status}`);
    }

    return res.json();
}
