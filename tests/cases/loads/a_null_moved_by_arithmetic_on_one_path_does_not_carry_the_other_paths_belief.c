int f(int k, int **pp) {
    if (!pp) return 0;
    int *p = 0;
    p = p + 1;
    if (k) p = *pp;
    if (!p) return 0;
    return *p;
}
