int count;
int tick(void) {
    return 1;
}
int main(void) {
    count = tick();
    count = count + tick();
    return count;
}
