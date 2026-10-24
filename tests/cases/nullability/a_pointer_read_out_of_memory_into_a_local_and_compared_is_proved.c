int f(int **pp) {
    if (pp) {
        int *q = *pp;
        if (q != 0) {
            return *q;
        }
    }
    return 0;
}
