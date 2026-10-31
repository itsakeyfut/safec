int main(void) {
    int *m[2];
    int x = 1;
    int i = 0;
    while (i < 2) {
        if (m[0]) {
            i++;
        }
        m[0] = &x;
        i++;
    }
    return 0;
}
