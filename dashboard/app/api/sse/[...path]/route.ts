import {type NextRequest} from 'next/server';
import {cookies} from 'next/headers';

const BACKEND_URL = process.env.BACKEND_URL ?? 'http://localhost:8080';

export const dynamic = 'force-dynamic';

export async function GET(request: NextRequest, {params}: {params: Promise<{path: string[]}>}) {
    const cookieStore = await cookies();
    const token = cookieStore.get('access_token')?.value;

    if (!token) {
        return new Response('Unauthorized', {status: 401});
    }

    const {path} = await params;
    const url = new URL(`${BACKEND_URL}/api/v1/${path.join('/')}`);

    request.nextUrl.searchParams.forEach((value, key) => {
        url.searchParams.set(key, value);
    });

    const upstream = await fetch(url.toString(), {
        headers: {
            Authorization: `Bearer ${token}`,
            Accept: 'text/event-stream',
            'Cache-Control': 'no-cache',
        },
        // @ts-expect-error Node fetch signal type
        signal: request.signal,
    });

    if (!upstream.ok || !upstream.body) {
        return new Response('SSE upstream failed', {status: upstream.status});
    }

    return new Response(upstream.body, {
        status: 200,
        headers: {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
            Connection: 'keep-alive',
            'X-Accel-Buffering': 'no',
        },
    });
}
