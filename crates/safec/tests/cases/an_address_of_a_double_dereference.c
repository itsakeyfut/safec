int f(int **pp) {
    int *r = &**pp;
    return *r;
}
