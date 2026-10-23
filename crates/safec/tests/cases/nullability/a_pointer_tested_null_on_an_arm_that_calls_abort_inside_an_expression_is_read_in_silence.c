void abort(void);
int *maybe(void);

int f(void) {
    int *p = maybe();
    int x = 0;
    if (p == 0) {
        x = (abort(), 0);
    }
    return *p + x;
}
