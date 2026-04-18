import {NextResponse, type NextRequest} from 'next/server';

const BACKEND_URL = process.env.BACKEND_URL ?? 'http://localhost:8080';

export async function POST(request: NextRequest) {
    const body = await request.json();

    const res = await fetch(`${BACKEND_URL}/api/v1/auth/login`, {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify(body),
    });

    if (!res.ok) {
        const data = await res.json();
        return NextResponse.json(data, {status: res.status});
    }

    const data = await res.json();

    const response = NextResponse.json({user: data.user});

    // Store tokens in httpOnly cookies
    response.cookies.set('access_token', data.access_token, {
        httpOnly: true,
        secure: process.env.NODE_ENV === 'production',
        sameSite: 'strict',
        maxAge: 3600, // 1 hour
        path: '/',
    });

    response.cookies.set('refresh_token', data.refresh_token, {
        httpOnly: true,
        secure: process.env.NODE_ENV === 'production',
        sameSite: 'strict',
        maxAge: 30 * 24 * 3600, // 30 days
        path: '/',
    });

    return response;
}
