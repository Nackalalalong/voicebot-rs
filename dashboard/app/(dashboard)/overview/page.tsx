'use client';

import {useQuery} from '@tanstack/react-query';
import {apiFetch} from '@/lib/api';

export default function OverviewPage() {
    const {data: campaigns} = useQuery({
        queryKey: ['campaigns'],
        queryFn: () => apiFetch('/api/v1/campaigns'),
    });

    return (
        <div>
            <h1 className="text-2xl font-semibold mb-6">Overview</h1>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <StatCard label="Total Campaigns" value={campaigns?.total ?? '—'} />
                <StatCard label="Active Campaigns" value="—" />
                <StatCard label="Calls Today" value="—" />
            </div>
        </div>
    );
}

function StatCard({label, value}: {label: string; value: string | number}) {
    return (
        <div className="bg-white rounded-lg shadow p-5">
            <p className="text-sm text-gray-500">{label}</p>
            <p className="text-3xl font-bold mt-1">{value}</p>
        </div>
    );
}
