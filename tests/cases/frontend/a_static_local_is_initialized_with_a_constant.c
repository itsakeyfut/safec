int f(int n) {
    static int kept = n;
    return kept;
}
