int f(int n) {
    int s = 0;
    for (int i = 0; i < n; i = i + 1) {
        int t = i;
        s = s + t;
    }
    return s;
}
