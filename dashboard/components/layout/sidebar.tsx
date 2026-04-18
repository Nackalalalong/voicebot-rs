'use client';

import Link from 'next/link';
import {usePathname, useRouter} from 'next/navigation';
import {LayoutDashboard, Megaphone, Phone, Settings, LogOut} from 'lucide-react';

const NAV_ITEMS = [
    {href: '/overview', label: 'Overview', icon: LayoutDashboard},
    {href: '/campaigns', label: 'Campaigns', icon: Megaphone},
    {href: '/calls', label: 'Calls', icon: Phone},
    {href: '/settings', label: 'Settings', icon: Settings},
];

export function Sidebar() {
    const pathname = usePathname();
    const router = useRouter();

    async function handleLogout() {
        await fetch('/api/auth/logout', {method: 'POST'});
        router.push('/login');
    }

    return (
        <aside className="w-56 bg-gray-900 text-white flex flex-col shrink-0">
            <div className="px-5 py-5 border-b border-gray-800">
                <span className="font-semibold text-lg">Voicebot</span>
            </div>
            <nav className="flex-1 py-4 px-3 space-y-1">
                {NAV_ITEMS.map(({href, label, icon: Icon}) => {
                    const active = pathname.startsWith(href);
                    return (
                        <Link
                            key={href}
                            href={href}
                            className={`flex items-center gap-3 px-3 py-2 rounded text-sm transition-colors ${
                                active
                                    ? 'bg-gray-700 text-white'
                                    : 'text-gray-400 hover:bg-gray-800 hover:text-white'
                            }`}>
                            <Icon size={16} />
                            {label}
                        </Link>
                    );
                })}
            </nav>
            <div className="p-3 border-t border-gray-800">
                <button
                    onClick={handleLogout}
                    className="flex items-center gap-3 px-3 py-2 rounded text-sm text-gray-400 hover:bg-gray-800 hover:text-white w-full transition-colors">
                    <LogOut size={16} />
                    Sign out
                </button>
            </div>
        </aside>
    );
}
