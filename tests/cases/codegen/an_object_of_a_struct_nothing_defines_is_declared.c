struct Opaque;
extern struct Opaque handle;
struct Opaque *address(void) {
    return &handle;
}
int main(void) {
    return address() == 0;
}
