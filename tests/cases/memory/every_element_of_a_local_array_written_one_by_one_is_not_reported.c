int main(void) {
    int a = 1;
    int b = 2;
    int *m[2];
    m[0] = &a;
    m[1] = &b;
    int *q = m[0];
    int *r = m[1];
    return *q + *r;
}
