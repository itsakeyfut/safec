int f(int x) {
    static int n = sizeof((int[]){x, 2});
    return n;
}
