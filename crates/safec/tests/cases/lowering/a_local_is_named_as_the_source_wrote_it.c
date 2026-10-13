int g(int);
int f(int p) {
    int n = g(p);
    {
        int n = 1;
        p = n;
    }
    return n + p;
}
