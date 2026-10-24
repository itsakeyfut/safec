void free(void *p);

int f(int **a, int **pp) {
    int *q = *pp;
    free(q);
    q = *pp;
    return *q + (a != 0);
}
