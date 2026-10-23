void abort(void);
int *maybe(void);

int f(void) {
    int *p = maybe();
    p != 0 || (abort(), 0);
    return *p;
}
