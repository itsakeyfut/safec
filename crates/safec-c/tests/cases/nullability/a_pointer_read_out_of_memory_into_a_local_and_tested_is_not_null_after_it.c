int f(int **pp) {
    if (pp) {
        int *q = *pp;
        if (q) {
            return *q;
        }
    }
    return 0;
}
