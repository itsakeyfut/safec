int f(int a[][4]) {
    return 0;
}

int main(void) {
    int a[4];
    int (*pa)[3] = &a;
    int m[2][3];
    return f(m);
}
