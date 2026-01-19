import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, PieChart, Pie, Cell } from 'recharts';
import { Clock, Calendar, CalendarDays, Users, Zap, TrendingUp, RefreshCw } from 'lucide-react';

interface TokenStatsAggregated {
    period: string;
    total_input_tokens: number;
    total_output_tokens: number;
    total_tokens: number;
    request_count: number;
}

interface AccountTokenStats {
    account_email: string;
    total_input_tokens: number;
    total_output_tokens: number;
    total_tokens: number;
    request_count: number;
}

interface TokenStatsSummary {
    total_input_tokens: number;
    total_output_tokens: number;
    total_tokens: number;
    total_requests: number;
    unique_accounts: number;
}

type TimeRange = 'hourly' | 'daily' | 'weekly';

const COLORS = ['#3b82f6', '#8b5cf6', '#ec4899', '#f59e0b', '#10b981', '#06b6d4', '#6366f1', '#f43f5e'];

const formatNumber = (num: number): string => {
    if (num >= 1000000) return `${(num / 1000000).toFixed(1)}M`;
    if (num >= 1000) return `${(num / 1000).toFixed(1)}K`;
    return num.toString();
};

const TokenStats: React.FC = () => {
    const { t } = useTranslation();
    const [timeRange, setTimeRange] = useState<TimeRange>('daily');
    const [chartData, setChartData] = useState<TokenStatsAggregated[]>([]);
    const [accountData, setAccountData] = useState<AccountTokenStats[]>([]);
    const [summary, setSummary] = useState<TokenStatsSummary | null>(null);
    const [loading, setLoading] = useState(true);

    const fetchData = async () => {
        setLoading(true);
        try {
            let hours = 24;
            let data: TokenStatsAggregated[] = [];

            switch (timeRange) {
                case 'hourly':
                    hours = 24;
                    data = await invoke<TokenStatsAggregated[]>('get_token_stats_hourly', { hours: 24 });
                    break;
                case 'daily':
                    hours = 168;
                    data = await invoke<TokenStatsAggregated[]>('get_token_stats_daily', { days: 7 });
                    break;
                case 'weekly':
                    hours = 720;
                    data = await invoke<TokenStatsAggregated[]>('get_token_stats_weekly', { weeks: 4 });
                    break;
            }

            setChartData(data);

            const [accounts, summaryData] = await Promise.all([
                invoke<AccountTokenStats[]>('get_token_stats_by_account', { hours }),
                invoke<TokenStatsSummary>('get_token_stats_summary', { hours })
            ]);

            setAccountData(accounts);
            setSummary(summaryData);
        } catch (error) {
            console.error('Failed to fetch token stats:', error);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
    }, [timeRange]);

    const pieData = accountData.slice(0, 8).map((account, index) => ({
        name: account.account_email.split('@')[0] + '...',
        value: account.total_tokens,
        fullEmail: account.account_email,
        color: COLORS[index % COLORS.length]
    }));

    return (
        <div className="p-6 space-y-6 max-w-7xl mx-auto">
            <div className="flex items-center justify-between">
                <h1 className="text-2xl font-bold text-gray-800 dark:text-white flex items-center gap-2">
                    <Zap className="w-6 h-6 text-blue-500" />
                    {t('token_stats.title', 'Token 消费统计')}
                </h1>
                <div className="flex items-center gap-2">
                    <div className="flex bg-gray-100 dark:bg-gray-800 rounded-lg p-1">
                        <button
                            onClick={() => setTimeRange('hourly')}
                            className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors flex items-center gap-1.5 ${
                                timeRange === 'hourly'
                                    ? 'bg-white dark:bg-gray-700 text-blue-600 shadow-sm'
                                    : 'text-gray-600 dark:text-gray-400 hover:text-gray-800'
                            }`}
                        >
                            <Clock className="w-4 h-4" />
                            {t('token_stats.hourly', '小时')}
                        </button>
                        <button
                            onClick={() => setTimeRange('daily')}
                            className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors flex items-center gap-1.5 ${
                                timeRange === 'daily'
                                    ? 'bg-white dark:bg-gray-700 text-blue-600 shadow-sm'
                                    : 'text-gray-600 dark:text-gray-400 hover:text-gray-800'
                            }`}
                        >
                            <Calendar className="w-4 h-4" />
                            {t('token_stats.daily', '日')}
                        </button>
                        <button
                            onClick={() => setTimeRange('weekly')}
                            className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors flex items-center gap-1.5 ${
                                timeRange === 'weekly'
                                    ? 'bg-white dark:bg-gray-700 text-blue-600 shadow-sm'
                                    : 'text-gray-600 dark:text-gray-400 hover:text-gray-800'
                            }`}
                        >
                            <CalendarDays className="w-4 h-4" />
                            {t('token_stats.weekly', '周')}
                        </button>
                    </div>
                    <button
                        onClick={fetchData}
                        disabled={loading}
                        className="p-2 rounded-lg bg-blue-500 text-white hover:bg-blue-600 transition-colors disabled:opacity-50"
                    >
                        <RefreshCw className={`w-4 h-4 ${loading ? 'animate-spin' : ''}`} />
                    </button>
                </div>
            </div>

            {summary && (
                <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                    <div className="bg-white dark:bg-gray-800 rounded-xl p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                        <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 text-sm mb-1">
                            <Zap className="w-4 h-4" />
                            {t('token_stats.total_tokens', '总 Token')}
                        </div>
                        <div className="text-2xl font-bold text-gray-800 dark:text-white">
                            {formatNumber(summary.total_tokens)}
                        </div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-xl p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                        <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 text-sm mb-1">
                            <TrendingUp className="w-4 h-4" />
                            {t('token_stats.input_tokens', '输入 Token')}
                        </div>
                        <div className="text-2xl font-bold text-blue-600">
                            {formatNumber(summary.total_input_tokens)}
                        </div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-xl p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                        <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 text-sm mb-1">
                            <TrendingUp className="w-4 h-4 rotate-180" />
                            {t('token_stats.output_tokens', '输出 Token')}
                        </div>
                        <div className="text-2xl font-bold text-purple-600">
                            {formatNumber(summary.total_output_tokens)}
                        </div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-xl p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                        <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 text-sm mb-1">
                            <Users className="w-4 h-4" />
                            {t('token_stats.accounts_used', '活跃账号')}
                        </div>
                        <div className="text-2xl font-bold text-green-600">
                            {summary.unique_accounts}
                        </div>
                    </div>
                </div>
            )}

            <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
                <div className="lg:col-span-2 bg-white dark:bg-gray-800 rounded-xl p-6 shadow-sm border border-gray-200 dark:border-gray-700">
                    <h2 className="text-lg font-semibold text-gray-800 dark:text-white mb-4">
                        {t('token_stats.usage_trend', 'Token 使用趋势')}
                    </h2>
                    <div className="h-64">
                        {chartData.length > 0 ? (
                            <ResponsiveContainer width="100%" height="100%">
                                <BarChart data={chartData}>
                                    <CartesianGrid strokeDasharray="3 3" vertical={false} stroke="#e5e7eb" />
                                    <XAxis
                                        dataKey="period"
                                        tick={{ fontSize: 11, fill: '#6b7280' }}
                                        tickFormatter={(val) => {
                                            if (timeRange === 'hourly') return val.split(' ')[1] || val;
                                            if (timeRange === 'daily') return val.split('-').slice(1).join('/');
                                            return val;
                                        }}
                                    />
                                    <YAxis
                                        tick={{ fontSize: 11, fill: '#6b7280' }}
                                        tickFormatter={(val) => formatNumber(val)}
                                    />
                                    <Tooltip
                                        formatter={(value: number) => [formatNumber(value), 'Tokens']}
                                        contentStyle={{
                                            borderRadius: '8px',
                                            border: 'none',
                                            boxShadow: '0 4px 6px -1px rgb(0 0 0 / 0.1)'
                                        }}
                                    />
                                    <Bar dataKey="total_input_tokens" name="Input" fill="#3b82f6" radius={[4, 4, 0, 0]} />
                                    <Bar dataKey="total_output_tokens" name="Output" fill="#8b5cf6" radius={[4, 4, 0, 0]} />
                                </BarChart>
                            </ResponsiveContainer>
                        ) : (
                            <div className="h-full flex items-center justify-center text-gray-400">
                                {loading ? t('common.loading', '加载中...') : t('token_stats.no_data', '暂无数据')}
                            </div>
                        )}
                    </div>
                </div>

                <div className="bg-white dark:bg-gray-800 rounded-xl p-6 shadow-sm border border-gray-200 dark:border-gray-700">
                    <h2 className="text-lg font-semibold text-gray-800 dark:text-white mb-4">
                        {t('token_stats.by_account', '分账号统计')}
                    </h2>
                    <div className="h-48">
                        {pieData.length > 0 ? (
                            <ResponsiveContainer width="100%" height="100%">
                                <PieChart>
                                    <Pie
                                        data={pieData}
                                        cx="50%"
                                        cy="50%"
                                        innerRadius={40}
                                        outerRadius={70}
                                        paddingAngle={2}
                                        dataKey="value"
                                    >
                                        {pieData.map((entry, index) => (
                                            <Cell key={`cell-${index}`} fill={entry.color} />
                                        ))}
                                    </Pie>
                                    <Tooltip
                                        formatter={(value: number, _name: string, props: any) => [
                                            formatNumber(value),
                                            props.payload.fullEmail
                                        ]}
                                    />
                                </PieChart>
                            </ResponsiveContainer>
                        ) : (
                            <div className="h-full flex items-center justify-center text-gray-400">
                                {loading ? t('common.loading', '加载中...') : t('token_stats.no_data', '暂无数据')}
                            </div>
                        )}
                    </div>
                    <div className="mt-4 space-y-2 max-h-32 overflow-y-auto">
                        {accountData.slice(0, 5).map((account, index) => (
                            <div key={account.account_email} className="flex items-center justify-between text-sm">
                                <div className="flex items-center gap-2">
                                    <div
                                        className="w-3 h-3 rounded-full"
                                        style={{ backgroundColor: COLORS[index % COLORS.length] }}
                                    />
                                    <span className="text-gray-600 dark:text-gray-300 truncate max-w-[120px]">
                                        {account.account_email.split('@')[0]}
                                    </span>
                                </div>
                                <span className="font-medium text-gray-800 dark:text-white">
                                    {formatNumber(account.total_tokens)}
                                </span>
                            </div>
                        ))}
                    </div>
                </div>
            </div>

            {accountData.length > 0 && (
                <div className="bg-white dark:bg-gray-800 rounded-xl p-6 shadow-sm border border-gray-200 dark:border-gray-700">
                    <h2 className="text-lg font-semibold text-gray-800 dark:text-white mb-4">
                        {t('token_stats.account_details', '账号详细统计')}
                    </h2>
                    <div className="overflow-x-auto">
                        <table className="w-full text-sm">
                            <thead>
                                <tr className="border-b border-gray-200 dark:border-gray-700">
                                    <th className="text-left py-3 px-4 font-medium text-gray-500 dark:text-gray-400">
                                        {t('token_stats.account', '账号')}
                                    </th>
                                    <th className="text-right py-3 px-4 font-medium text-gray-500 dark:text-gray-400">
                                        {t('token_stats.requests', '请求数')}
                                    </th>
                                    <th className="text-right py-3 px-4 font-medium text-gray-500 dark:text-gray-400">
                                        {t('token_stats.input', '输入')}
                                    </th>
                                    <th className="text-right py-3 px-4 font-medium text-gray-500 dark:text-gray-400">
                                        {t('token_stats.output', '输出')}
                                    </th>
                                    <th className="text-right py-3 px-4 font-medium text-gray-500 dark:text-gray-400">
                                        {t('token_stats.total', '合计')}
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {accountData.map((account) => (
                                    <tr
                                        key={account.account_email}
                                        className="border-b border-gray-100 dark:border-gray-700/50 hover:bg-gray-50 dark:hover:bg-gray-700/30"
                                    >
                                        <td className="py-3 px-4 text-gray-800 dark:text-white">
                                            {account.account_email}
                                        </td>
                                        <td className="py-3 px-4 text-right text-gray-600 dark:text-gray-300">
                                            {account.request_count.toLocaleString()}
                                        </td>
                                        <td className="py-3 px-4 text-right text-blue-600">
                                            {formatNumber(account.total_input_tokens)}
                                        </td>
                                        <td className="py-3 px-4 text-right text-purple-600">
                                            {formatNumber(account.total_output_tokens)}
                                        </td>
                                        <td className="py-3 px-4 text-right font-semibold text-gray-800 dark:text-white">
                                            {formatNumber(account.total_tokens)}
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                </div>
            )}
        </div>
    );
};

export default TokenStats;
