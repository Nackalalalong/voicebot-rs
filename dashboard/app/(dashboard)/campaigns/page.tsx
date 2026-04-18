'use client';

import {useQuery} from '@tanstack/react-query';
import Link from 'next/link';
import {apiFetch} from '@/lib/api';

export default function CampaignsPage() {
    const {data, isLoading} = useQuery({
        queryKey: ['campaigns'],
        queryFn: () => apiFetch('/api/v1/campaigns'),
    });

    return (
        <div>
            <div className="flex items-center justify-between mb-6">
                <h1 className="text-2xl font-semibold">Campaigns</h1>
                <Link
                    href="/campaigns/new"
                    className="bg-blue-600 text-white px-4 py-2 rounded text-sm hover:bg-blue-700">
                    New campaign
                </Link>
            </div>

            {isLoading ? (
                <p className="text-gray-500">Loading…</p>
            ) : (
                <div className="bg-white rounded-lg shadow overflow-hidden">
                    <table className="w-full text-sm">
                        <thead className="bg-gray-50 text-gray-600 text-left">
                            <tr>
                                <th className="px-4 py-3">Name</th>
                                <th className="px-4 py-3">Status</th>
                                <th className="px-4 py-3">Language</th>
                                <th className="px-4 py-3">Created</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y">
                            {data?.items?.map((c: any) => (
                                <tr key={c.id} className="hover:bg-gray-50">
                                    <td className="px-4 py-3">
                                        <Link
                                            href={`/campaigns/${c.id}`}
                                            className="text-blue-600 hover:underline">
                                            {c.name}
                                        </Link>
                                    </td>
                                    <td className="px-4 py-3">
                                        <StatusBadge status={c.status} />
                                    </td>
                                    <td className="px-4 py-3">{c.language}</td>
                                    <td className="px-4 py-3">
                                        {new Date(c.created_at).toLocaleDateString()}
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

function StatusBadge({status}: {status: string}) {
    const colors: Record<string, string> = {
        active: 'bg-green-100 text-green-800',
        draft: 'bg-gray-100 text-gray-800',
        paused: 'bg-yellow-100 text-yellow-800',
        completed: 'bg-blue-100 text-blue-800',
        archived: 'bg-red-100 text-red-800',
    };
    return (
        <span
            className={`px-2 py-0.5 rounded text-xs font-medium ${colors[status] ?? 'bg-gray-100'}`}>
            {status}
        </span>
    );
}
