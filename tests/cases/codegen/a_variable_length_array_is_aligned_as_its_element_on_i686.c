struct pair {
    long long first;
    char second;
};

long long sized(int n) {
    long long wide[n];
    struct pair pairs[n];
    wide[0] = 1;
    pairs[0].first = 2;
    return wide[0] + pairs[0].first + (long long)sizeof wide + (long long)sizeof pairs;
}
