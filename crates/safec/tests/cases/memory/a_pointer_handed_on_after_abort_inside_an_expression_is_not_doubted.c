void abort(void);
void use(int *p);

int f(int n, int *q) {
    int *p = q;
    n > 0 || (abort(), 0);
    use(p);
    return 0;
}
