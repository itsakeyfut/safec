// On the arm where the test found it null, the parameter is null, and
// `realloc` of it returns fresh memory: the arm keeps the doubt (#931).
void *realloc(void *p, int n);
void free(void *p);
int f(int **p) {
    if (!p) {
        int **q = realloc(p, 16);
        if (!q) return 0;
        int *v = q[0];
        free(q);
        return v != 0;
    }
    return 0;
}
