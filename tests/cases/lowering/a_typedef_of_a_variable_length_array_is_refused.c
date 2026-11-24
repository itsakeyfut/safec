int sized(int n) {
    typedef int T[n];
    n = n + 1;
    T a;
    return (int)sizeof a;
}
