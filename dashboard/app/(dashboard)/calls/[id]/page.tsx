'use client';

import {useQuery} from '@tanstack/react-query';
import {useParams} from 'next/navigation';
import Link from 'next/link';
import {apiFetch} from '@/lib/api';

export default function CallDetailPage() {
    const {id} = useParams<{id: string}>();

    const {data: call, isLoading} = useQuery({
        queryKey: ['calls', id],
        queryFn: () => apiFetch(`/api/v1/calls/${id}`),
    });

    if (isLoading) return <p className="text-gray-500">Loading…</p>;
    if (!call) return <p className="text-red-500">Call not found</p>;

    const transcript: {role: string; text: string; ts?: number}[] = call.transcript ?? [];

    return (
        <div className="max-w-3xl">
            <div className="mb-4">
                <Link href="/calls" className="text-gray-400 hover:text-gray-600 text-sm">
                    ← Calls
                </Link>
            </div>

            <div className="flex items-center gap-4 mb-6">
                <h1 className="text-2xl font-semibold font-mono">{call.phone_number}</h1>
                <span className="text-sm text-gray-500 capitalize">{call.direction}</span>
                <span className="text-sm text-gray-500 capitalize">{call.status}</span>
            </div>

            {/* Metadata */}
            <div className="bg-white rounded-lg shadow p-4 mb-4 grid grid-cols-2 gap-4 text-sm">
                <div>
                    <p className="text-xs text-gray-500">Duration</p>
                    <p className="font-medium">{call.duration_secs != null ? `${call.duration_secs}s` : '—'}</p>
                </div>
                <div>
                    <p className="text-xs text-gray-500">Date</p>
                    <p className="font-medium">{new Date(call.created_at).toLocaleString()}</p>
                </div>
                <div>
                    <p className="text-xs text-gray-500">Sentiment</p>
                    <p className="font-medium capitalize">{call.sentiment ?? '—'}</p>
                </div>
                <div>
                    <p className="text-xs text-gray-500">Recording</p>
                    {call.recording_url ? (
                        <audio controls src={call.recording_url} className="h-8 mt-1" />
                    ) : (
                        <p className="font-medium">—</p>
                    )}
                </div>
            </div>

            {/* Transcript */}
            <div className="bg-white rounded-lg shadow p-4">
                <h2 className="text-sm font-semibold mb-3">Transcript</h2>
                {transcript.length === 0 ? (
                    <p className="text-gray-400 text-sm">No transcript available.</p>
                ) : (
                    <div className="space-y-3">
                        {transcript.map((msg, i) => (
                            <div
                                key={i}
                                className={`flex gap-3 ${msg.role === 'assistant' ? 'justify-start' : 'justify-end'}`}>
                                <div
                                    className={`max-w-[80%] rounded-lg px-3 py-2 text-sm ${
                                        msg.role === 'assistant'
                                            ? 'bg-gray-100 text-gray-800'
                                            : 'bg-blue-600 text-white'
                                    }`}>
                                    <p className="text-xs opacity-60 mb-1 capitalize">{msg.role}</p>
                                    {msg.text}
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            {/* Custom Metrics */}
            {call.custom_metrics && (
                <div className="bg-white rounded-lg shadow p-4 mt-4">
                    <h2 className="text-sm font-semibold mb-2">Custom Metrics</h2>
                    <pre className="text-xs bg-gray-50 rounded p-3 overflow-auto">
                        {JSON.stringify(call.custom_metrics, null, 2)}
                    </pre>
                </div>
            )}
        </div>
    );
}
