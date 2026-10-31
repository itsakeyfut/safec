int main(void) {
    int *m[2];
    int x = 1;
    int i = 0;
    while (m[0] == 0 || i < 2) {
        m[0] = &x;
        i++;
    }
    return 0;
}
