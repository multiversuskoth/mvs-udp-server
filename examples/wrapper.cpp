// Declare the Rust function
extern "C" void start_rollback_server_cpp();

int main() {
    // This will exit immediately but when called from 
    // a program that has its own loop(UE game loop) then it should be fine
    start_rollback_server_cpp();
}
