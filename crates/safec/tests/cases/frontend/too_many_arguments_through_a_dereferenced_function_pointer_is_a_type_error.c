int f(int (*fp)(int)) {
    return (*fp)(1, 2);
}
