void free(void *p);
int **get(void);

int f(void) {
    int **r = get();
    int *q = *r;
    free(q);
    q = *r;
    return *q;
}
