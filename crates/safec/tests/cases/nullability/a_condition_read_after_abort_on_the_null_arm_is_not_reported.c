void abort(void);
int *maybe(void);

int f(void) {
    int *p = maybe();
    int x = 0;
    if (p == 0) {
        if ((abort(), *p)) {
            x = 1;
        }
    }
    return x;
}
