void *malloc(int n);
void f(char **cc) {
    if (cc == 0) {
        return;
    }
    char *c = *cc;
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = c;
    void *pm = m;
    int **pi = pm;
    int *p = *pi;
    if (p != 0) {
        *p = 1;
    }
}
int main(void) {
    char c;
    char *pc = &c;
    f(&pc);
    return 0;
}
