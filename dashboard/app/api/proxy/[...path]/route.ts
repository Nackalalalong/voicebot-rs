import {NextResponse, type NextRequest} from 'next/server';
import {cookies} from 'next/headers';

const BACKEND_URL = process.env.BACKEND_URL ?? 'http://localhost:8080';

export async function GET(request: NextRequest, {params}: {params: Promise<{path: string[]}>}) {
    return proxyRequest(request, await params);
}

export async function POST(request: NextRequest, {params}: {params: Promise<{path: string[]}>}) {
    return proxyRequest(request, await params);
}

export async function PUT(request: NextRequest, {params}: {params: Promise<{path: string[]}>}) {
    return proxyRequest(request, await params);
}

export async function DELETE(request: NextRequest, {params}: {params: Promise<{path: string[]}>}) {
    return proxyRequest(request, await params);
}

async function proxyRequest(request: NextRequest, params: {path: string[]}) {
    const cookieStore = await cookies();
    const token = cookieStore.get('access_token')?.value;

    if (!token) {
        return NextResponse.json({error: 'unauthorized'}, {status: 401});
    }

    const path = params.path.join('/');
    const url = new URL(`${BACKEND_URL}/api/v1/${path}`);

    // Forward query parameters
    request.nextUrl.searchParams.forEach((value, key) => {
        url.searchParams.set(key, value);
    });

    const headers: HeadersInit = {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
    };

    const body =
        request.method !== 'GET' && request.method !== 'DELETE' ? await request.text() : undefined;

    const res = await fetch(url.toString(), {
        method: request.method,
        headers,
        body,
    });

    const data = await res.text();
    return new NextResponse(data, {
        status: res.status,
        headers: {'Content-Type': 'application/json'},
    });
}
