int initialized(void) {
    const char m[] = "yo";
    const char padded[8] = "yo";
    return m[0] + padded[7];
}

int written(void) {
    const char m[] = "yo";
    ((char *)m)[0] = 'x';
    return m[0];
}
