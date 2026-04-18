import type {NextConfig} from 'next';

const nextConfig: NextConfig = {
    output: 'standalone',
    experimental: {
        serverActions: {
            allowedOrigins: ['localhost:3000'],
        },
    },
    // Proxy API calls to Rust backend in development
    async rewrites() {
        return [];
    },
};

export default nextConfig;
