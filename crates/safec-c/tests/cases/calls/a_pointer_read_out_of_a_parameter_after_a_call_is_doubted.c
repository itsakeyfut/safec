void release(void);

int f(int **pp) {
    release();
    int *q = *pp;
    return *q;
}
