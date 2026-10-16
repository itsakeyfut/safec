void free(void *p);
int **get(void);
int **get_other(void);

int f(int c) {
    int **r;
    if (c) {
        r = get();
    } else {
        r = get_other();
    }
    int *q = *r;
    free(q);
    q = *r;
    return *q;
}
