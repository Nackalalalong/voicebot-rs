'use client';

import {useQuery} from '@tanstack/react-query';
import {apiFetch} from '@/lib/api';

export default function CallsPage() {
    const {data, isLoading} = useQuery({
        queryKey: ['calls'],
        queryFn: () => apiFetch('/api/v1/calls'),
    });

    return (
        <div>
            <h1 className="text-2xl font-semibold mb-6">Call Records</h1>
            {isLoading ? (
                <p className="text-gray-500">Loading…</p>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left">
                            <tr>
                                <th className="px-4 py-3">Phone</th>
                                <th className="px-4 py-3">Direction</th>
                                <th className="px-4 py-3">Status</th>
                                <th className="px-4 py-3">Duration</th>
                                <th className="px-4 py-3">Date</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {data?.items?.map((c: any) => (
                                <tr key={c.id}>
                                    <td className="px-4 py-3 font-mono">{c.phone_number}</td>
                                    <td className="px-4 py-3 capitalize">{c.direction}</td>
                                    <td className="px-4 py-3 capitalize">{c.status}</td>
                                    <td className="px-4 py-3">
                                        {c.duration_secs != null ? `${c.duration_secs}s` : '—'}
                                    </td>
                                    <td className="px-4 py-3">
                                        {new Date(c.created_at).toLocaleString()}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}
