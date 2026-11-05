int main(void) {
    int x = 1;
    int *q;
    for (int i = 0; i < 2; i++) {
        q = &x;
    }
    int *r = q;
    return r == 0;
}
