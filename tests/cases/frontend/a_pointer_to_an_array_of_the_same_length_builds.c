int f(int a[][4]) {
    return a[0][0];
}

int main(void) {
    int a[4];
    int (*pa)[4] = &a;
    int m[2][4];
    m[0][0] = 1;
    return f(m) + (*pa)[0];
}
