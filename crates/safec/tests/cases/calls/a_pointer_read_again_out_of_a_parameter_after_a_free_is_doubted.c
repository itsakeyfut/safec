void free(void *p);

int f(int **pp) {
    int *q = *pp;
    free(q);
    q = *pp;
    return *q;
}
